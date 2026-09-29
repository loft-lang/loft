// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A7.2 — `cdylib` native extension loader tests.
//!
//! Tests the manifest `native` field, `pending_native_libs` propagation on
//! Parser, and the `extensions::load_all()` dispatch path.

extern crate loft;

use loft::manifest::{Manifest, read_manifest};
use loft::parser::Parser;
use std::sync::Mutex;

mod common;
use common::cached_default;

// Native extension tests share global state (NATIVE_REGISTRY, LOADED_LIBS) so they
// must run sequentially within this test binary.  The stub set is NOT among them any
// more — it lives on `State`, see
// `a_sibling_compile_does_not_take_over_this_program_s_stub_set` below.
static TEST_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// A7.2.1: manifest `native` field is parsed and accessible
// ---------------------------------------------------------------------------

/// A7.2.1: `read_manifest` returns the `native` field from `[library]`.
#[test]
fn manifest_parses_native_field() {
    use std::io::Write;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("loft_a72_test_{}.toml", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"[package]\nloft = \">=0.8\"\n\n[library]\nnative = \"loft_myext\"\n")
        .unwrap();
    let m: Manifest = read_manifest(path.to_str().unwrap()).unwrap();
    assert_eq!(m.native.as_deref(), Some("loft_myext"));
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// A7.2.2: Parser accumulates pending_native_libs when a manifest has `native`
// ---------------------------------------------------------------------------

/// A7.2.2: Parser resolves the native library path when a package manifest
/// declares `native = "..."`.  The path is only added to `pending_native_libs`
/// when the pre-built `.so` exists or `auto_build_native` succeeds.
/// The test fixture has no buildable native crate, so the list stays empty —
/// but parsing must still succeed without errors.
#[test]
fn parser_native_pkg_parses_without_error() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_str("pub use native_pkg::*;", "test", false);
    // No parse errors — the #native stub is registered even without the .so.
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(
        !has_errors,
        "unexpected errors: {:?}",
        p.diagnostics.lines()
    );
}

// ---------------------------------------------------------------------------
// A7.2.3: extensions::load_one registers functions via loft_register_v1
// ---------------------------------------------------------------------------

/// Helper: resolve the test fixture cdylib path.  Returns None if not built.
///
/// Also verifies the cdylib is fresher than its Rust source.  Neither
/// `cargo test` nor `make ci` rebuilds this `.so`; a stale artefact
/// after a core-layout change (e.g. the C54 Phase 2c i32→i64 element
/// stride swap) silently masquerades as a vector-marshalling
/// regression.  On stale detection we panic with the rebuild command.
fn fixture_lib_path() -> Option<String> {
    let path = if cfg!(target_os = "macos") {
        "tests/lib/native_pkg/native/target/release/libloft_native_test.dylib"
    } else if cfg!(windows) {
        "tests/lib/native_pkg/native/target/release/loft_native_test.dll"
    } else {
        "tests/lib/native_pkg/native/target/release/libloft_native_test.so"
    };
    let p = std::path::Path::new(path);
    if !p.exists() {
        return None;
    }
    let src = std::path::Path::new("tests/lib/native_pkg/native/src/lib.rs");
    if let (Ok(art_md), Ok(src_md)) = (p.metadata(), src.metadata())
        && let (Ok(art_mtime), Ok(src_mtime)) = (art_md.modified(), src_md.modified())
        && src_mtime > art_mtime
    {
        panic!(
            "stale fixture cdylib — source newer than artefact:\n  \
               source:   {} (mtime={:?})\n  \
               artefact: {} (mtime={:?})\n\
             Rebuild:\n  \
               (cd tests/lib/native_pkg/native && cargo build --release)\n",
            src.display(),
            src_mtime,
            p.display(),
            art_mtime,
        );
    }
    Some(path.to_string())
}

/// A7.2.3: `extensions::load_one` loads a cdylib and registers its functions.
///
/// Requires the fixture shared library to be pre-built.
/// Build with: `cd tests/lib/native_pkg/native && cargo build --release`
#[test]
fn load_one_registers_native_functions() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
            );
            return;
        }
    };

    let native_decl = r#"
pub fn ext_add_one(x: integer) -> integer;
#native "loft_ext_add_one"
"#;
    let source = r#"
fn main() {
    assert(ext_add_one(41) == 42, "ext_add_one(41) should be 42, got {ext_add_one(41)}")
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Load the fixture cdylib; it registers "n_ext_add_one" under "loft_ext_add_one".
    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);

    state.execute_argv("main", &p.data, &[]);
}

/// A `#native` fn declared with a NULLABLE scalar (`-> integer?`) must still
/// wire: `Optional(τ)` shares τ's sentinel layout, so the marshal signature is
/// the plain i64 and null (i64::MIN) crosses the boundary intact.  Pre-fix,
/// the signature classified as un-marshallable, the symbol was never wired,
/// and the call hit the stale-cdylib panic stub (found via loft-libs-core's
/// `random.rand -> integer?`, loft-libs-core#14).  Wide/negative values ride
/// the same i64 lane (`ext_echo`).
#[test]
fn wires_optional_integer_return_and_wide_values() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
            );
            return;
        }
    };

    let native_decl = r#"
pub fn ext_maybe(x: integer) -> integer?;
#native "loft_ext_maybe"
pub fn ext_echo(x: integer) -> integer;
#native "loft_ext_echo"
"#;
    let source = r#"
fn main() {
    assert(ext_maybe(41) == 42, "ext_maybe(41) should be 42, got {ext_maybe(41)}");
    n = ext_maybe(-1);
    assert(!n, "ext_maybe(-1) should be null (the i64::MIN sentinel must survive marshalling)");
    assert(ext_echo(1099511627776) == 1099511627776, "2^40 must round-trip untruncated");
    assert(ext_echo(-1099511627776) == -1099511627776, "-2^40 must round-trip untruncated");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);

    state.execute_argv("main", &p.data, &[]);
}

/// loft-lang/loft#409: a `vector<u8>` returned by an FFI bridge (built with
/// `alloc_vector_from_bytes` — the null-store alloc path the crypto/imaging
/// libs use) must survive an in-place `+=` by the caller.  Pre-fix the append
/// rebuilt a fresh EMPTY buffer and dropped the returned elements (len 4 → 1
/// instead of 5), because the wrapper forwarded the foreign store without
/// filling its `__retbuf`.  Covers the WRAPPER shape (`fn make { ext_make_bytes }`)
/// — the real library pattern (a private native + a public loft wrapper).
#[test]
fn ffi_returned_vector_survives_in_place_append_409() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Needs the fixture cdylib (loaded via `--lib`); skip cleanly when absent.
    if fixture_lib_path().is_none() {
        eprintln!(
            "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
        );
        return;
    }

    // Invoke the real binary and assert on captured stdout — the loft `assert`
    // does NOT propagate out of an in-process `execute_argv`, so an in-process
    // check would pass vacuously (the positive-control trap).
    let prog = std::env::temp_dir().join("loft_409_ffi_vec_append.loft");
    std::fs::write(
        &prog,
        "use native_pkg::*;\n\
         fn make(n: integer) -> vector<u8> { ext_make_bytes(n) }\n\
         fn main() { v = make(4); v += [99 as u8]; println(\"R={len(v)} {v[0]} {v[4]}\"); }\n",
    )
    .expect("write program");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--lib")
        .arg("tests/lib/native_pkg")
        .arg("--interpret")
        .arg(&prog)
        .output()
        .expect("run loft binary");
    let _ = std::fs::remove_file(&prog);

    let stdout = String::from_utf8_lossy(&out.stdout);
    // len 4 returned + 1 appended = 5; first byte of [0,1,2,3] survives; appended = 99.
    assert!(
        stdout.contains("R=5 0 99"),
        "loft#409: an FFI-returned vector<u8> must survive an in-place `+=` (expected \
         `R=5 0 99`); got stdout: {stdout:?}, stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// loft-lang/loft#410 (sibling of #409): the DIRECT pub-`#native`-decl shape.
/// `ext_make_bytes` is declared `pub fn … -> vector<u8>; #native "…"` and
/// called WITHOUT a loft wrapper, so the result is a foreign-store FFI return
/// bound straight to the caller's local.  #409 fixed only the WRAPPER shape
/// (the result flowed through a loft fn's `__retbuf`); a direct call has no
/// wrapper, so the local borrowed the foreign store with no owned `__vdb`
/// buffer and the in-place `+=` rebuilt a fresh EMPTY one — dropping the
/// returned elements (len 4 → 1).  The fix materialises the foreign return
/// into an owned buffer at the ASSIGNMENT (parser/expressions.rs), routing it
/// through a named `__fwd` local so the foreign source store is also freed
/// (no per-assignment leak).
#[test]
fn ffi_returned_vector_direct_decl_survives_in_place_append_410() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if fixture_lib_path().is_none() {
        eprintln!(
            "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
        );
        return;
    }

    // No wrapper fn — `ext_make_bytes` (the `#native` decl) is called directly.
    let prog = std::env::temp_dir().join("loft_410_ffi_vec_direct.loft");
    std::fs::write(
        &prog,
        "use native_pkg::*;\n\
         fn main() { v = ext_make_bytes(4); v += [99 as u8]; println(\"R={len(v)} {v[0]} {v[4]}\"); }\n",
    )
    .expect("write program");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--lib")
        .arg("tests/lib/native_pkg")
        .arg("--interpret")
        .arg(&prog)
        .output()
        .expect("run loft binary");
    let _ = std::fs::remove_file(&prog);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // len 4 returned + 1 appended = 5; first byte of [0,1,2,3] survives; appended = 99.
    assert!(
        stdout.contains("R=5 0 99"),
        "loft#410: a DIRECT `#native`-decl FFI vector<u8> return must survive an in-place \
         `+=` (expected `R=5 0 99`); got stdout: {stdout:?}, stderr: {stderr:?}"
    );
    // The materialise must also free the foreign source store (the #409
    // `__fwd` discipline) — no orphaned-store leak warning at exit.
    assert!(
        !stderr.contains("stores not freed"),
        "loft#410: materialising the FFI return must not leak the foreign source store; \
         got stderr: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// A7.2.4: registry takes priority over dlsym — issue #119
// ---------------------------------------------------------------------------

/// A7.2.4: When a cdylib registers `n_fn` under the name `"loft_fn"` via
/// `loft_register_v1`, AND also exports a raw C-ABI `loft_fn` symbol,
/// the registered version must be used — not the dlsym fallback.
///
/// The fixture cdylib exports:
/// - `n_ext_add_one(x) -> x + 1`   (registered as "loft_ext_add_one")
/// - `loft_ext_add_one(x) -> x + 1000`  (raw C-ABI export, dlsym bait)
///
/// If the registry wins: `ext_add_one(41) == 42`.
/// If dlsym wins:         `ext_add_one(41) == 1041`.
#[test]
fn registry_takes_priority_over_dlsym() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
            );
            return;
        }
    };

    let native_decl = r#"
pub fn ext_add_one(x: integer) -> integer;
#native "loft_ext_add_one"
"#;
    // The assertion checks that the registered version (x+1) is called,
    // not the dlsym fallback (x+1000).
    let source = r#"
fn main() {
    result = ext_add_one(41);
    assert(result == 42, "Issue #119: expected 42 (registry), got {result} (dlsym fallback used wrong function)");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    assert!(
        p.diagnostics.is_empty(),
        "diagnostics: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);

    // If we get here, the registered version (x+1) was wired correctly.
    state.execute_argv("main", &p.data, &[]);
}

// ---------------------------------------------------------------------------
// A7.2.5: guard panics when #native name is missing from registry — issue #119
// ---------------------------------------------------------------------------

/// A7.2.5: When a cdylib uses `loft_register_v1` but a `#native` annotation
/// refers to a symbol that wasn't registered, AND that symbol is found via
/// dlsym, `wire_native_fns` must panic — not silently use the wrong function.
///
/// This test runs as a subprocess to avoid corrupting the global static
/// registries (`NATIVE_REGISTRY`, `STUB_SYMBOLS`, `NATIVE_SIGS`) that are
/// shared across tests in the same process.
#[test]
fn guard_catches_unregistered_dlsym_fallback() {
    if fixture_lib_path().is_none() {
        eprintln!("skipping: fixture cdylib not built");
        return;
    }

    // Run ourselves as a subprocess with a special env var to trigger
    // the inner test logic.
    if std::env::var("LOFT_TEST_GUARD_INNER").is_ok() {
        guard_inner();
        return;
    }

    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe)
        .env("LOFT_TEST_GUARD_INNER", "1")
        .arg("guard_catches_unregistered_dlsym_fallback")
        .arg("--exact")
        .arg("--test-threads=1")
        .arg("--nocapture")
        .output()
        .expect("failed to spawn subprocess");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "subprocess should have panicked but exited successfully"
    );
    assert!(
        stderr.contains("was not registered via loft_register_v1"),
        "expected registration bug panic message, got:\n{stderr}"
    );
}

fn guard_inner() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let lib_path = fixture_lib_path().unwrap();

    // Use a #native name that is NOT registered by loft_register_v1,
    // but IS exported as a raw C-ABI symbol (dlsym will find it).
    let native_decl = r#"
pub fn ext_bad(x: integer) -> integer;
#native "loft_ext_unregistered"
"#;
    let source = r#"
fn main() {
    println("{ext_bad(1)}");
}
"#;
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    extensions::load_all(&mut state, vec![lib_path]);
    // This should panic with the registration bug message.
    extensions::wire_native_fns(&mut state, &p.data);
}

// ---------------------------------------------------------------------------
// A7.3: Vector marshalling patterns
// ---------------------------------------------------------------------------

/// Helper: run a loft program with the test native library loaded.
fn run_native_test(native_decl: &str, source: &str) {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let lib_path = match fixture_lib_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping: fixture cdylib not built");
            return;
        }
    };

    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(!has_errors, "diagnostics: {:?}", p.diagnostics.lines());
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    extensions::load_all(&mut state, vec![lib_path]);
    extensions::wire_native_fns(&mut state, &p.data);
    state.execute_argv("main", &p.data, &[]);
}

/// A7.3.1: vector<integer> passed to native function — sum should be correct.
#[test]
fn vec_i32_sum() {
    run_native_test(
        r#"
pub fn ext_vec_sum(data: vector<integer>) -> integer;
#native "loft_ext_vec_sum"
"#,
        r#"
fn main() {
    data = [10, 20, 30, 40];
    result = ext_vec_sum(data);
    assert(result == 100, "vec_sum: expected 100, got {result}");
}
"#,
    );
}

/// A7.3.2: vector<single> (f32) passed to native function.
#[test]
fn vec_f32_sum() {
    run_native_test(
        r#"
pub fn ext_vec_sum_f32(data: vector<single>) -> integer;
#native "loft_ext_vec_sum_f32"
"#,
        r#"
fn main() {
    data = [1.0f, 2.0f, 3.0f, 4.0f];
    result = ext_vec_sum_f32(data);
    assert(result == 10, "vec_sum_f32: expected 10, got {result}");
}
"#,
    );
}

/// A7.3.3: scalar before vector parameter.
#[test]
fn scalar_before_vec() {
    run_native_test(
        r#"
pub fn ext_offset_sum(offset: integer, data: vector<integer>) -> integer;
#native "loft_ext_offset_sum"
"#,
        r#"
fn main() {
    data = [1, 2, 3];
    result = ext_offset_sum(100, data);
    assert(result == 106, "offset_sum: expected 106, got {result}");
}
"#,
    );
}

/// A7.3.4: vector between two scalars.
#[test]
fn vec_between_scalars() {
    run_native_test(
        r#"
pub fn ext_sandwich_sum(a: integer, data: vector<integer>, b: integer) -> integer;
#native "loft_ext_sandwich_sum"
"#,
        r#"
fn main() {
    data = [10, 20];
    result = ext_sandwich_sum(1, data, 2);
    assert(result == 33, "sandwich_sum: expected 33, got {result}");
}
"#,
    );
}

/// A7.3.5: vector from struct field (indirect reference).
#[test]
fn vec_from_struct_field() {
    run_native_test(
        r#"
pub fn ext_struct_vec_len(data: vector<integer>) -> integer;
#native "loft_ext_struct_vec_len"

struct TestBox {
    items: vector<integer>
}
"#,
        r#"
fn main() {
    b = TestBox { items: [1, 2, 3, 4, 5] };
    result = ext_struct_vec_len(b.items);
    assert(result == 5, "struct_vec_len: expected 5, got {result}");
}
"#,
    );
}

/// A7.3.6: vector call inside if block inside loop — issue #120 pattern.
#[test]
fn vec_in_loop_if() {
    run_native_test(
        r#"
pub fn ext_loop_vec_sum(data: vector<integer>) -> integer;
#native "loft_ext_loop_vec_sum"
"#,
        r#"
fn main() {
    data = [5, 10, 15];
    total = 0;
    for i in 0..10 {
        if true {
            s = ext_loop_vec_sum(data);
            total += s;
        }
    }
    assert(total == 300, "loop_vec_sum: expected 300, got {total}");
}
"#,
    );
}

/// A7.3.7: vector from struct field of a RETURNED struct.
/// This is the textured-cube pattern: make_texture() returns a Canvas,
/// then gl_upload_canvas(canvas.data, ...) reads the vector.
#[test]
fn vec_from_returned_struct() {
    run_native_test(
        r#"
pub fn ext_struct_vec_len(data: vector<integer>) -> integer;
#native "loft_ext_struct_vec_len"

struct TestBox {
    items: vector<integer>
}

fn make_box() -> TestBox {
    TestBox { items: [10, 20, 30, 40, 50, 60, 70, 80] }
}
"#,
        r#"
fn main() {
    b = make_box();
    result = ext_struct_vec_len(b.items);
    assert(result == 8, "returned_struct_vec: expected 8, got {result}");
}
"#,
    );
}

/// A7.3.8: vector from struct field of returned struct, with other calls between.
/// Tests that the store isn't freed/reused between make_box() and ext_struct_vec_len().
#[test]
fn vec_from_returned_struct_with_gap() {
    run_native_test(
        r#"
pub fn ext_add_one(x: integer) -> integer;
#native "loft_ext_add_one"

pub fn ext_struct_vec_len(data: vector<integer>) -> integer;
#native "loft_ext_struct_vec_len"

struct TestBox {
    items: vector<integer>
}

fn make_box() -> TestBox {
    TestBox { items: [1, 2, 3, 4] }
}
"#,
        r#"
fn main() {
    b = make_box();
    dummy = ext_add_one(0);
    dummy = ext_add_one(1);
    dummy = ext_add_one(2);
    result = ext_struct_vec_len(b.items);
    assert(result == 4, "returned_struct_gap: expected 4, got {result}");
}
"#,
    );
}

/// A7.3.9: vector from returned struct after heavy allocation.
/// Simulates the make_texture() pattern: create a struct with a large vector,
/// do many operations that allocate temporary stores, then return the struct.
#[test]
fn vec_from_returned_struct_heavy() {
    run_native_test(
        r#"
pub fn ext_vec_sum(data: vector<integer>) -> integer;
#native "loft_ext_vec_sum"

struct BigBox {
    width: integer,
    height: integer,
    data: vector<integer>
}

fn make_big() -> BigBox {
    w = 4;
    h = 4;
    d: vector<integer> = [];
    for y in 0..h {
        for x in 0..w {
            d += [x + y * w];
        }
    }
    BigBox { width: w, height: h, data: d }
}
"#,
        r#"
fn main() {
    b = make_big();
    assert(b.width == 4, "width: {b.width}");
    assert(b.height == 4, "height: {b.height}");
    result = ext_vec_sum(b.data);
    expected = 0;
    for i in 0..16 { expected += i; }
    assert(result == expected, "heavy: expected {expected}, got {result}");
}
"#,
    );
}

/// A7.3.9 (continued)
#[test]
fn vec_struct_field_in_loop() {
    run_native_test(
        r#"
pub fn ext_loop_vec_sum(data: vector<integer>) -> integer;
#native "loft_ext_loop_vec_sum"

struct Container {
    vals: vector<integer>
}
"#,
        r#"
fn main() {
    c = Container { vals: [3, 7] };
    total = 0;
    for i in 0..5 {
        total += ext_loop_vec_sum(c.vals);
    }
    assert(total == 50, "struct_field_loop: expected 50, got {total}");
}
"#,
    );
}

/// A second program compiled in the SAME PROCESS must not decide which symbols the
/// first one is allowed to wire.
///
/// The stub set — the `#native` symbols a compile registered a panic stub for, and
/// therefore the only ones `wire_native_fns` may replace — used to live in a
/// process-global that `compile::byte_code` OVERWROTE wholesale on every compile.  So
/// in any process that compiles more than one program (a test binary, the REPL loading
/// a second file, an embedder), a sibling compile landing between one program's
/// compile and its wiring replaced the set.  The wiring then hit
/// `!stubs.contains(sym) → continue`, skipped resolution for its OWN symbols, and left
/// the panic stub in place — surfacing much later, at the first call, as *"native
/// function not loaded: its library's native cdylib is missing or stale"*.  That
/// message sends the reader after a build problem that does not exist.
///
/// It cost `tests/repl_session.rs::file_debugger_can_call_into_a_native_library` a ~50 %
/// failure rate: it passes alone and fails about half the time beside its 54 siblings,
/// measured at 5/10 in a worktree A/B against the preceding commit.
///
/// This reproduces that interleaving DETERMINISTICALLY — compile B, compile a sibling,
/// then wire and run B — so the guard fails outright on the old code instead of
/// depending on thread scheduling.  The sibling declares a DIFFERENT `#native` symbol,
/// which is the whole point: a shared set would now hold only the sibling's.
#[test]
fn a_sibling_compile_does_not_take_over_this_program_s_stub_set() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let Some(lib_path) = fixture_lib_path() else {
        eprintln!(
            "skipping: fixture cdylib not built — run: cd tests/lib/native_pkg/native && cargo build --release"
        );
        return;
    };

    // Program B — the one that gets wired and run.
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

    // The sibling compile, landing between B's compile and B's wiring.  Its stub set
    // names a symbol B does not use, so a process-global set would no longer contain
    // `loft_ext_add_one` and B's own wiring would skip it.
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

    // Wire B and run it.  Pre-fix this panicked in the stub with the stale-cdylib
    // message; the program's own `assert` is the value check.
    extensions::load_all(&mut state_b, vec![lib_path]);
    extensions::wire_native_fns(&mut state_b, &pb.data);
    state_b.execute_argv("main", &pb.data, &[]);
}
