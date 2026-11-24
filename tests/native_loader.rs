// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN225 R3 — a package's own native library is refused.  A `[library] native` cdylib that
//! cargo built and the interpreter loaded, or one the package shipped prebuilt, was the last
//! route by which a library resolved to Rust (@PLN206 P6, route 3).  It is gone: a package
//! whose `loft.toml` declares `[library] native`, `[native] crate` or their tables is refused
//! at that declaration, on every backend, naming the cure (a `[c] shim`), and nothing is
//! built or loaded for it.  A component loft ships (`[native] component`) is loft's own
//! library and still loads (`tests/ffi_c_library.rs`).
//!
//! Also here: the stub set a compile registers belongs to its `State`, not the process.

extern crate loft;

use loft::file_access as fa;
use loft::parser::Parser;
use std::path::PathBuf;

mod common;
use common::cached_default;

/// A scratch library tree holding package `rustlib`, whose manifest declares `manifest_tail`
/// after its `[library] entry`, with a Rust crate on disk a regression would build.
fn rust_native_package(tag: &str, manifest_tail: &str) -> PathBuf {
    let libs = std::env::temp_dir().join(format!("loft_r3_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&libs);
    let pkg = libs.join("rustlib");
    fa::create_dir_all(pkg.join("src")).expect("the package directory");
    fa::create_dir_all(pkg.join("native/src")).expect("the crate directory");
    fa::write(
        pkg.join("loft.toml"),
        format!(
            "[package]\nname = \"rustlib\"\nversion = \"0.1.0\"\n\n\
             [library]\nentry = \"src/rustlib.loft\"\n{manifest_tail}"
        ),
    )
    .expect("the manifest");
    fa::write(
        pkg.join("src/rustlib.loft"),
        "pub fn answer() -> integer;\n#native \"n_rustlib_answer\"\n",
    )
    .expect("the source");
    fa::write(
        pkg.join("native/Cargo.toml"),
        "[package]\nname = \"loft-rustlib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n",
    )
    .expect("the crate manifest");
    fa::write(
        pkg.join("native/src/lib.rs"),
        "#[unsafe(no_mangle)]\npub extern \"C\" fn n_rustlib_answer() -> i64 { 42 }\n",
    )
    .expect("the crate source");
    libs
}

/// The backends a program is run on: the interpreter and the C backend (`--native`), plus an
/// explicit `--native-c` where the C runtime has its POSIX host layer.
fn backends() -> Vec<&'static str> {
    let mut b = vec!["--interpret", "--native"];
    if loft::platform::c_runtime_posix_host() {
        b.push("--native-c");
    }
    b
}

/// The R3 guard: a program using a package with a Rust native part is refused on every
/// backend, at the manifest line that declares it, with the message naming the C part, and
/// nothing is built in the package (`native/target` stays absent).  That no `cargo` or `rustc`
/// is called is `tests/lib_rustc_free.rs`'s (its stubs are shell scripts, Unix only).
#[test]
fn a_package_s_own_native_library_is_refused_on_every_backend() {
    let libs = rust_native_package(
        "refused",
        "native = \"loft_rustlib\"\n\n[native]\ncrate = \"loft-rustlib\"\n",
    );
    let program = libs.join("prog.loft");
    fa::write(
        &program,
        "use rustlib;\nfn main() { println(\"{rustlib::answer()}\"); }\n",
    )
    .expect("the program");
    for backend in backends() {
        let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&program)
            .current_dir(&libs)
            .env("LOFT_TIMEOUT", "300")
            .env("LOFT_NO_CACHE", "1");
        let out = cmd.output().expect("loft runs");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success(),
            "{backend}: the program must not run\nstdout: {}\nstderr: {stderr}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            stderr.contains(loft::manifest::RUST_NATIVE_REFUSED),
            "{backend}: the refusal names the package's native library and the C part\n{stderr}"
        );
        assert!(
            stderr.contains("loft.toml:7"),
            "{backend}: the refusal points at the `[library] native` line\n{stderr}"
        );
        assert!(
            !fa::exists(libs.join("rustlib/native/target")),
            "{backend}: nothing was built for the package"
        );
    }
    let _ = fa::remove_dir_all(&libs);
}

/// Each declaration of the package's own native library is refused on its own: a
/// `[native] crate` with no `[library] native` (a crate the compiled backends linked), and the
/// `[native.functions]` table.  The parse reports the refusal at the declaration's line.
#[test]
fn every_declaration_of_a_rust_native_part_is_refused() {
    for (tag, tail, line) in [
        ("crate", "\n[native]\ncrate = \"loft-rustlib\"\n", 9),
        (
            "functions",
            "\n[native.functions]\nanswer = \"n_rustlib_answer\"\n",
            9,
        ),
    ] {
        let libs = rust_native_package(tag, tail);
        let mut p = Parser::new();
        let (data, db) = cached_default();
        p.data = data;
        p.database = db;
        p.lib_dirs.push(libs.to_string_lossy().into_owned());
        p.parse_str(
            "use rustlib;\nfn main() { x = rustlib::answer(); }\n",
            "test",
            false,
        );
        let lines = p.diagnostics.lines();
        let _ = fa::remove_dir_all(&libs);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(loft::manifest::RUST_NATIVE_REFUSED)
                    && l.contains(&format!("loft.toml:{line}"))),
            "{tag}: refused at its declaration: {lines:?}"
        );
        assert!(
            p.data.native_packages.is_empty(),
            "{tag}: no native package was registered: {:?}",
            p.data.native_packages
        );
    }
}

/// The stub set a compile registers belongs to its own `State`: a sibling compile between
/// one program's compile and its wiring used to replace a process-global set, so the first
/// program's natives were no longer recognised as stubs.  The sibling declares a DIFFERENT
/// `#native` symbol, which is the whole point: a shared set would now hold only the sibling's.
#[test]
fn a_sibling_compile_does_not_take_over_this_program_s_stub_set() {
    use loft::compile::byte_code;
    use loft::scopes;
    use loft::state::State;

    // Program B — the one whose stub set is checked.
    let mut pb = Parser::new();
    let (data, db) = cached_default();
    pb.data = data;
    pb.database = db;
    pb.parse_str(
        "pub fn ext_add_one(x: integer) -> integer;\n#native \"loft_ext_add_one\"\n",
        "b_decl",
        false,
    );
    pb.parse_str(
        "fn main() {\n    assert(ext_add_one(41) == 42, \"ext_add_one(41) should be 42\")\n}\n",
        "b_main",
        false,
    );
    assert!(pb.diagnostics.is_empty(), "B: {:?}", pb.diagnostics.lines());
    scopes::check(&mut pb.data, &mut pb.database);
    let mut state_b = State::new(pb.database.clone());
    byte_code(&mut state_b, &mut pb.data);

    // The sibling compile, landing after B's compile.
    let mut pa = Parser::new();
    let (data_a, db_a) = cached_default();
    pa.data = data_a;
    pa.database = db_a;
    pa.parse_str(
        "pub fn sibling_only(x: integer) -> integer;\n#native \"loft_sibling_only\"\n",
        "a_decl",
        false,
    );
    pa.parse_str("fn main() {\n    x = 1;\n}\n", "a_main", false);
    scopes::check(&mut pa.data, &mut pa.database);
    let mut state_a = State::new(pa.database.clone());
    byte_code(&mut state_a, &mut pa.data);

    assert!(
        state_b.native_stub_symbols.contains("loft_ext_add_one"),
        "B's stub set must survive a sibling compile, got {:?}",
        state_b.native_stub_symbols
    );
    assert!(
        !state_b.native_stub_symbols.contains("loft_sibling_only"),
        "B's stub set must not take the sibling's symbol, got {:?}",
        state_b.native_stub_symbols
    );
}
