// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 Arc N / N2 — the auto-generated native-library cdylib must COMPILE
//! and be DISPATCHABLE from an interpreted script.
//!
//! `native_lib::generate_cdylib_lib_rs` produces a cdylib `lib.rs` from a
//! library's scalar-dispatchable functions (the `--native` program + export
//! wrappers).  Two milestones:
//!
//! 1. `generated_cdylib_compiles_and_exports_scalar_symbol` — the generated
//!    source actually builds as a `cdylib` against `libloft.rlib`, and the export
//!    symbol (`loft_n_double`) is present.
//! 2. `dispatches_scalar_call_into_generated_cdylib` — an interpreted script that
//!    declares `double` as `#native "loft_n_double"` and calls `double(21)`
//!    dispatches into the generated cdylib and gets `42` — the full
//!    interpret→native store-ABI round trip for the scalar slice.  (Auto-deriving
//!    the `#native` decl from `use <lib>` is N3 policy; here it's written by hand
//!    to isolate the dispatch mechanism.)

extern crate loft;

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

mod common;
use common::cached_default;

// load_all / wire_native_fns mutate process-global registries (NATIVE_REGISTRY,
// STUB_SYMBOLS, LOADED_LIBS); byte_code calls set_stub_symbols.  Serialise both
// tests so one's stub set can't clobber the other's between wire and execute.
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Locate `libloft.rlib` + its sibling `deps/` for standalone rustc, matching the
/// feature set of this test binary (mirrors `tests/native.rs::find_loft_rlib`).
fn find_loft_rlib() -> Option<(PathBuf, PathBuf)> {
    let deps = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let rlib = std::fs::read_dir(&deps)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("libloft-") || n == "libloft.rlib") && n.ends_with(".rlib")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?
        .path();
    Some((rlib, deps))
}

/// `--extern name=path` for optional feature deps (random/png) that the generated
/// stdlib code may reference, mirroring `tests/native.rs::collect_extra_externs`.
fn extra_externs(deps: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(deps) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.starts_with("lib") || !n.ends_with(".rlib") || n.starts_with("libloft") {
                continue;
            }
            if let Some(stem) = n
                .strip_prefix("lib")
                .and_then(|s| s.rsplit_once('-'))
                .map(|x| x.0)
            {
                out.push((stem.to_string(), e.path()));
            }
        }
    }
    out
}

/// Platform cdylib filename for `stem`.
fn cdylib_name(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// Compile `src` as a cdylib against `libloft.rlib`, mirroring the `--native`
/// rustc invocation.  Panics (keeping the source for inspection) on failure.
fn compile_cdylib(src: &str, stem: &str, tmp: &Path, rlib: &Path, deps: &Path) -> PathBuf {
    let rs = tmp.join(format!("{stem}.rs"));
    fa::write(&rs, src).unwrap();
    let so = tmp.join(cdylib_name(stem));
    let mut args: Vec<String> = vec![
        "--edition=2024".into(),
        "-C".into(),
        "debuginfo=0".into(),
        "-C".into(),
        "opt-level=0".into(),
        "--crate-type".into(),
        "cdylib".into(),
        "-o".into(),
        so.display().to_string(),
        rs.display().to_string(),
        "--extern".into(),
        format!("loft={}", rlib.display()),
        "-L".into(),
        deps.display().to_string(),
    ];
    for (name, path) in extra_externs(deps) {
        args.push("--extern".into());
        args.push(format!("{name}={}", path.display()));
    }
    // Windows MSVC: add the build-script `-L` dirs holding native import libs
    // (`windows.0.48.5.lib` etc.) or the link fails LNK1181.  No-op off Windows.
    // (`build_shared_cdylib` does the same for the production path.)
    for dir in common::native_lib_search_dirs(rlib) {
        args.push("-L".into());
        args.push(dir.display().to_string());
    }
    // Pass args via an `@argfile`: the `--extern`/`-L` list exceeds Windows'
    // ~32 KB CreateProcessW command-line limit (os error 206); argfile is
    // cross-platform (mirrors `build_shared_cdylib` + the --native test runner).
    let argfile = tmp.join(format!("{stem}.args"));
    let contents = args
        .iter()
        .map(|s| {
            if s.contains(char::is_whitespace) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fa::write(&argfile, contents).unwrap();
    let out = Command::new("rustc")
        .arg(format!("@{}", argfile.display()))
        .output()
        .expect("invoke rustc");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Surface an environment-failure hint (full TMPDIR / OOM) ABOVE the raw
    // rustc spam — a SIGBUS from `ld` writing to a full tmpfs otherwise reads
    // like a stale-cdylib or codegen bug. (Shared with loft's own pipeline.)
    let hint = loft::native_lib::toolchain_failure_hint(&stderr)
        .map(|h| format!("{h}\n\n"))
        .unwrap_or_default();
    assert!(
        out.status.success(),
        "{hint}cdylib compile FAILED. source at {}\n--- rustc stderr (tail) ---\n{}",
        rs.display(),
        stderr
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(fa::exists(&so), "cdylib output should exist");
    so
}

/// Build a cdylib from `lib_src` (a pure library — no `main`) exporting the
/// scalar-dispatchable function `fn_name` as `loft_n_<fn_name>`.  Returns
/// (so_path, tmp_dir), or None when the rlib/rustc/stdlib aren't available
/// (test skips).
fn build_scalar_lib_cdylib(stem: &str, lib_src: &str, fn_name: &str) -> Option<(PathBuf, PathBuf)> {
    let (rlib, deps) = find_loft_rlib()?;
    if Command::new("rustc").arg("--version").output().is_err() {
        println!("skip: rustc unavailable");
        return None;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n2_{stem}_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(lib_src, "lib", false);
    loft::scopes::check(&mut p.data, &mut p.database);

    let scalar = loft::native_gate::scalar_dispatchable(&p.data);
    let fn_nr = p.data.def_nr(&format!("n_{fn_name}"));
    assert!(
        scalar.contains(&fn_nr),
        "{fn_name} should be scalar-dispatchable"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(fn_nr).collect();

    // Compile to bytecode to populate the database (the type schema codegen reads).
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);

    let src = loft::native_lib::generate_cdylib_lib_rs(&p.data, &state.database, &export);
    let want = format!("pub extern \"C\" fn loft_n_{fn_name}");
    assert!(
        src.contains(&want),
        "the export wrapper for {fn_name} must be present"
    );

    let so = compile_cdylib(&src, stem, &tmp, &rlib, &deps);
    Some((so, tmp))
}

/// Build a cdylib from `lib_src` (a pure library — no `main`) exporting the
/// **shared-store-dispatchable** function `fn_name` as `loft_shared_n_<fn_name>`
/// (the `*mut Stores` bridge, for a non-scalar value crossing the boundary).
/// Returns (so_path, tmp_dir), or None when the toolchain isn't available.
fn build_shared_lib_cdylib(stem: &str, lib_src: &str, fn_name: &str) -> Option<(PathBuf, PathBuf)> {
    let (rlib, deps) = find_loft_rlib()?;
    if Command::new("rustc").arg("--version").output().is_err() {
        println!("skip: rustc unavailable");
        return None;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n2_{stem}_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(lib_src, "lib", false);
    loft::scopes::check(&mut p.data, &mut p.database);

    let shared = loft::native_gate::shared_store_dispatchable(&p.data);
    let fn_nr = p.data.def_nr(&format!("n_{fn_name}"));
    assert!(
        shared.contains(&fn_nr),
        "{fn_name} should be shared-store-dispatchable"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(fn_nr).collect();

    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);

    let src = loft::native_lib::generate_shared_cdylib_lib_rs(&p.data, &state.database, &export);
    let want = format!("pub extern \"C\" fn loft_shared_n_{fn_name}");
    assert!(
        src.contains(&want),
        "the shared bridge for {fn_name} must be present"
    );

    let so = compile_cdylib(&src, stem, &tmp, &rlib, &deps);
    Some((so, tmp))
}

/// The canonical `double` library used by the compile + dispatch milestones.
fn build_double_cdylib(stem: &str) -> Option<(PathBuf, PathBuf)> {
    build_scalar_lib_cdylib(
        stem,
        "pub fn double(x: integer) -> integer { x * 2 }",
        "double",
    )
}

#[test]
fn generated_cdylib_compiles_and_exports_scalar_symbol() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_double_cdylib("loft_n2_compile") else {
        return;
    };
    assert!(fa::exists(&so), "cdylib output should exist");
    let _ = fa::remove_dir_all(&tmp);
}

/// @PLN26 phase 2 — a shared-store cdylib that calls a `[native] crate` package's
/// fn emits the **C-ABI** path: an `extern "C"` block with a `#[link_name]`'d
/// `__cabi_<sym>` decl (the package's `.so` is linked by `build_shared_cdylib`),
/// NOT `extern crate <pkg>` (the legacy rlib path that can't take two `loft_ffi`
/// rlibs into one cdylib).  Source-level (no rustc): proves the codegen half —
/// that `emit_program` turns `native_cabi` on for a cdylib — deterministically
/// and without a native build.  The link half + the live cdylib→native call are
/// covered by the integration path (a no-native-of-its-own library that `use`s a
/// `[native] crate` package, dispatched as a cdylib).
#[test]
fn shared_cdylib_with_native_package_emits_cabi_extern() {
    // The C-ABI path is the default on every host; only `LOFT_NATIVE_CABI=0`
    // forces the legacy rlib link (and then `build_shared_cdylib` refuses the
    // combination).  Skip under that escape hatch so the env can't flip the
    // asserted path.
    if std::env::var("LOFT_NATIVE_CABI").as_deref() == Ok("0") {
        println!("skip: LOFT_NATIVE_CABI=0 forces the legacy rlib path");
        return;
    }
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    // A body-less `#native` fn (the package symbol) + a shared-store-dispatchable
    // pub fn (vector arg + return) that calls it, so the native is reachable and
    // emitted into the cdylib.
    p.parse_str(
        "fn gl_clear(c: integer);\n\
         #native \"loft_gl_clear\"\n\
         \n\
         pub fn clear_grid(grid: vector<integer>, c: integer) -> vector<integer> {\n\
         \x20 gl_clear(c);\n\
         \x20 grid\n\
         }\n",
        "lib",
        false,
    );
    loft::scopes::check(&mut p.data, &mut p.database);
    // Register the symbol's package exactly as a `[native] crate` manifest would
    // (`parse_str` reads source, not a manifest): a non-empty `native_packages`
    // plus the symbol→crate mapping the codegen consults.
    p.data
        .native_packages
        .push(("loft-gl-native".to_string(), "/nonexistent/gl".to_string()));
    p.data
        .native_symbol_crates
        .insert("loft_gl_clear".to_string(), "loft_gl_native".to_string());

    let shared = loft::native_gate::shared_store_dispatchable(&p.data);
    let fn_nr = p.data.def_nr("n_clear_grid");
    assert!(
        shared.contains(&fn_nr),
        "clear_grid should be shared-store-dispatchable"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(fn_nr).collect();

    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    let src = loft::native_lib::generate_shared_cdylib_lib_rs(&p.data, &state.database, &export);

    // C-ABI path: an `extern "C"` block, the symbol declared under a `__cabi_`
    // alias bound by `#[link_name]` (so it never shadows the wrapper fn).
    assert!(
        src.contains("unsafe extern \"C\""),
        "expected an extern \"C\" block (the C-ABI native path):\n{src}"
    );
    assert!(
        src.contains("#[link_name = \"loft_gl_clear\"]"),
        "expected a #[link_name] decl for the native package symbol"
    );
    assert!(
        src.contains("__cabi_loft_gl_clear"),
        "expected the __cabi_-aliased native symbol"
    );
    // NOT the legacy rlib path (which would bring a second `loft_ffi` rlib into
    // the cdylib → duplicate `loft_register_v1`, unlinkable).
    assert!(
        !src.contains("extern crate loft_gl_native"),
        "must NOT take the rlib `extern crate` path under the C-ABI default"
    );
}

/// The C-ABI extern decl must state the cdylib's REAL integer width: a plain
/// loft `integer` is i64 at the package boundary (the same @P370 judgment the
/// interpreter marshal uses — `forced_size`, not value range), and a nullable
/// `integer?` peels to the same i64 (Optional shares the sentinel layout).
/// Pre-fix the decl said `i32`, silently truncating i64 traffic — the null
/// sentinel (i64::MIN) arrived as 0 (loft-libs-core#14, `random.rand`).
#[test]
fn cabi_extern_declares_i64_for_plain_and_optional_integers() {
    if std::env::var("LOFT_NATIVE_CABI").as_deref() == Ok("0") {
        println!("skip: LOFT_NATIVE_CABI=0 forces the legacy rlib path");
        return;
    }
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    // A body-less `#native` decl with plain-integer params and a NULLABLE
    // integer return, called from a shared-store-dispatchable pub fn so the
    // native is reachable and the extern block is emitted.
    p.parse_str(
        "fn roll(lo: integer, hi: integer) -> integer?;\n\
         #native \"loft_roll\"\n\
         \n\
         pub fn roll_grid(grid: vector<integer>) -> vector<integer> {\n\
         \x20 roll(1, 6);\n\
         \x20 grid\n\
         }\n",
        "lib",
        false,
    );
    loft::scopes::check(&mut p.data, &mut p.database);
    p.data.native_packages.push((
        "loft-roll-native".to_string(),
        "/nonexistent/roll".to_string(),
    ));
    p.data
        .native_symbol_crates
        .insert("loft_roll".to_string(), "loft_roll_native".to_string());

    let shared = loft::native_gate::shared_store_dispatchable(&p.data);
    let fn_nr = p.data.def_nr("n_roll_grid");
    assert!(
        shared.contains(&fn_nr),
        "roll_grid should be shared-store-dispatchable (its callee's `integer?` peels to a marshallable i64)"
    );
    let export: std::collections::HashSet<u32> = std::iter::once(fn_nr).collect();

    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    let src = loft::native_lib::generate_shared_cdylib_lib_rs(&p.data, &state.database, &export);

    assert!(
        src.contains("fn __cabi_loft_roll(lo: i64, hi: i64) -> i64;"),
        "extern decl must be i64 throughout (plain integer params, Optional-integer return):\n{src}"
    );
}

// Plan-74: the scalar-slice DISPATCH tests are gone.  They drove a
// zero-registration scalar cdylib (`generate_cdylib_lib_rs` output, raw
// `extern "C"` exports, no `loft_register_bridges_v1`) through the legacy
// ~98-arm raw-ptr marshaller, which has been removed.  The scalar codegen is
// still compile-tested by `generated_cdylib_compiles_and_exports_scalar_symbol`
// above; the live production dispatch is the **shared-store bridge** path
// (`generate_shared_cdylib_lib_rs` → `shared_store_dispatch`), exercised by the
// many `dispatches_*_into_shared_cdylib` tests below — and the registry-priority
// / unregistered-dlsym guard (#119) is covered by `tests/native_loader.rs`
// (A7.2.4 / A7.2.5) against the bridge-migrated `native_pkg` fixture.

/// The store-touching slice's library: `vec_sum` takes a `vector<integer>` (a
/// non-scalar value crossing the boundary) and returns its sum.
const VEC_SUM_LIB: &str = "pub fn vec_sum(data: vector<integer>) -> integer {\n\
                           \x20   total = 0;\n\
                           \x20   for x in data { total += x; }\n\
                           \x20   total\n\
                           }";

/// N2 store-touching slice, milestone 1: the shared-store bridge (`*mut Stores`
/// + `LibArg` ABI) for a `vector<integer>`-arg function must COMPILE.
#[test]
fn generated_shared_cdylib_compiles() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_c", VEC_SUM_LIB, "vec_sum")
    else {
        return;
    };
    assert!(fa::exists(&so), "shared cdylib output should exist");
    let _ = fa::remove_dir_all(&tmp);
}

/// Run a SCRIPT that declares `native_decl` (a `#native "loft_shared_…"` import
/// of an auto-generated shared-store bridge) and calls it from `main`,
/// dispatching into `so` over the shared `*mut Stores` ABI.
fn run_shared_dispatch(so: &Path, native_decl: &str, source: &str) {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(native_decl, "native_decl", false);
    p.parse_str(source, "test", false);
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(!has_errors, "diagnostics: {:?}", p.diagnostics.lines());
    scopes::check(&mut p.data, &mut p.database);

    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    extensions::wire_shared_native_fns(&mut state, &p.data);

    state.execute_argv("main", &p.data, &[]);
}

/// N2 store-touching slice, milestone 2: dispatch a `vector<integer>` argument
/// into the generated shared-store bridge end-to-end.  The interpreter builds the
/// vector in its store; the bridge shares that store by pointer; the `--native`
/// body sums it — `vec_sum([10,20,30]) == 60`.  The raw stack `DbRef` crosses
/// unchanged (no deref) and resolves correctly in the shared store.
#[test]
fn dispatches_vector_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_d", VEC_SUM_LIB, "vec_sum")
    else {
        return;
    };

    let native_decl = "pub fn vec_sum(data: vector<integer>) -> integer not null;\n#native \"loft_shared_n_vec_sum\"\n";
    let source = r#"
fn main() {
    d = [10, 20, 30];
    assert(vec_sum(d) == 60, "vec_sum([10,20,30]) should be 60, got {vec_sum(d)}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// A function that ALLOCATES a `vector<integer>` and returns it — the non-scalar
/// *return* case.  The `--native` body allocates in the shared store; the
/// returned `DbRef` must be valid back in the interpreter, which then iterates it.
const RANGE_VEC_LIB: &str = "pub fn range_vec(n: integer) -> vector<integer> {\n\
                             \x20   v: vector<integer> = [];\n\
                             \x20   for i in 0..n { v += [i]; }\n\
                             \x20   v\n\
                             }";

/// N2 store-touching slice, milestone 3: a non-scalar *return* crosses the
/// boundary.  `range_vec(4)` allocates `[0,1,2,3]` in the SHARED store inside the
/// native body and returns its `DbRef`; the interpreter sums it to 6.
///
/// `--native` returns a vector via a **hidden trailing destination `DbRef`**
/// (`Attribute::hidden`, appended by `ref_return`) that the caller pre-allocates
/// (`stores.null_named` + `OpDatabase(<type_id>)`).  The shared bridge wrapper
/// allocates that destination itself, so the script-side `#native` forward-decl
/// still models only the public param `n`.  Proves native-made store allocations
/// are valid back in the interpreter.
#[test]
fn dispatches_vector_return_from_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_r", RANGE_VEC_LIB, "range_vec")
    else {
        return;
    };

    let native_decl = "pub fn range_vec(n: integer) -> vector<integer> not null;\n#native \"loft_shared_n_range_vec\"\n";
    let source = r#"
fn main() {
    v = range_vec(4);
    total = 0;
    for x in v { total += x; }
    assert(total == 6, "sum of range_vec(4) = 0+1+2+3 should be 6, got {total}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// A function taking a struct `reference` (schema-DEPENDENT — the native body
/// reads `p.x`/`p.y` at `db.finish()`-computed field offsets).
const POINT_SUM_LIB: &str = "struct Point { x: integer, y: integer }\n\
                             pub fn point_sum(p: Point) -> integer { p.x + p.y }";

/// N2 store-touching probe: a struct `reference` ARG crosses the boundary.  This
/// is the first SCHEMA-DEPENDENT case — the library cdylib and the interpreter,
/// built from SEPARATE `Data`, must assign `Point` the same type id + field
/// offsets for the shared `DbRef` to read correctly.  `point_sum(Point{3,4}) == 7`.
#[test]
fn dispatches_struct_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_s", POINT_SUM_LIB, "point_sum")
    else {
        return;
    };

    let native_decl = "struct Point { x: integer, y: integer }\npub fn point_sum(p: Point) -> integer not null;\n#native \"loft_shared_n_point_sum\"\n";
    let source = r#"
fn main() {
    p = Point { x: 3, y: 4 };
    r = point_sum(p);
    assert(r == 7, "point_sum of x=3 y=4 should be 7, got {r}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// A function that constructs and RETURNS a struct.  Unlike a vector return,
/// `--native` does NOT use a hidden destination — the body allocates the record
/// fresh and returns its `DbRef` (`n_make_point(cell, a, b) -> DbRef`).
const MAKE_POINT_LIB: &str = "struct Point { x: integer, y: integer }\n\
                              pub fn make_point(a: integer, b: integer) -> Point {\n\
                              \x20   Point { x: a, y: b }\n\
                              }";

/// N2 store-touching: a struct `reference` RETURN crosses the boundary.  The
/// native body allocates a `Point` in the shared store and returns its `DbRef`;
/// the interpreter reads its fields.  `make_point(3,4)` → `p.x*10 + p.y == 34`.
#[test]
fn dispatches_struct_return_from_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) =
        build_shared_lib_cdylib("loft_n2_shared_sr", MAKE_POINT_LIB, "make_point")
    else {
        return;
    };

    let native_decl = "struct Point { x: integer, y: integer }\npub fn make_point(a: integer, b: integer) -> Point not null;\n#native \"loft_shared_n_make_point\"\n";
    let source = r#"
fn main() {
    p = make_point(3, 4);
    r = p.x * 10 + p.y;
    assert(r == 34, "make_point(3,4) fields should give 34, got {r}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// A function taking a `text` arg (passed as `&str` — ptr+len borrowed from the
/// shared store) and returning a scalar.
const STR_LEN_LIB: &str = "pub fn str_len(s: text) -> integer {\n\
                           \x20   n = 0;\n\
                           \x20   for c in s { n += 1; }\n\
                           \x20   n\n\
                           }";

/// N2 store-touching: a `text` ARG crosses the boundary.  `--native` takes a
/// `text` parameter as `&str` (ptr+len), not a `DbRef`; the bridge borrows the
/// store-backed bytes for the call.  `str_len("hello") == 5`.
#[test]
fn dispatches_text_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_t", STR_LEN_LIB, "str_len")
    else {
        return;
    };

    let native_decl =
        "pub fn str_len(s: text) -> integer not null;\n#native \"loft_shared_n_str_len\"\n";
    let source = r#"
fn main() {
    n = str_len("hello");
    assert(n == 5, "str_len('hello') should be 5, got {n}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// A function RETURNING `text` — `--native` uses the `text_return` `&mut String`
/// work buffer (hidden param) and returns a `Str`.  The bridge owns a local
/// `String`, then copies the result into the shared store's scratch.
const SHOUT_LIB: &str = "pub fn shout(s: text) -> text {\n\
                         \x20   s + \"!\"\n\
                         }";

/// N2 store-touching: a `text` RETURN crosses the boundary.  The native body
/// builds the result in a work `String`; the bridge copies it into the shared
/// store's scratch so it survives back in the interpreter.  `shout("hi") == "hi!"`.
#[test]
fn dispatches_text_return_from_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_shared_tr", SHOUT_LIB, "shout") else {
        return;
    };

    let native_decl = "pub fn shout(s: text) -> text not null;\n#native \"loft_shared_n_shout\"\n";
    let source = r#"
fn main() {
    r = shout("hi");
    assert(r == "hi!", "shout('hi') should be 'hi!', got {r}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// Plain (tag-only) enum: `--native` represents it as a `u8` tag (both arg and
/// return — no hidden dest), riding in the `LibArg` scalar slot.
const DIR_LIB: &str = "enum Direction { North, East, South, West }\n\
                       pub fn dir_code(d: Direction) -> integer {\n\
                       \x20   match d { North => 0, East => 1, South => 2, West => 3 }\n\
                       }\n\
                       pub fn dir_from(n: integer) -> Direction {\n\
                       \x20   if n == 1 { East } else { North }\n\
                       }";

/// N2 store-touching: a plain `enum` crosses the boundary as a `u8` tag, both as
/// an ARG (`dir_code(South) == 2`) and a RETURN (`dir_from(1) == East`, verified
/// by round-tripping through `dir_code`).
#[test]
fn dispatches_plain_enum_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // dir_code: enum arg, scalar return.
    let Some((so_c, tmp_c)) = build_shared_lib_cdylib("loft_n2_enum_c", DIR_LIB, "dir_code") else {
        return;
    };
    let decl_c = "enum Direction { North, East, South, West }\npub fn dir_code(d: Direction) -> integer not null;\n#native \"loft_shared_n_dir_code\"\n";
    run_shared_dispatch(
        &so_c,
        decl_c,
        "fn main() { c = dir_code(South); assert(c == 2, \"dir_code(South) should be 2, got {c}\") }",
    );
    let _ = fa::remove_dir_all(&tmp_c);

    // dir_from: scalar arg, enum return — verify by feeding it back to dir_code.
    let Some((so_f, tmp_f)) = build_shared_lib_cdylib("loft_n2_enum_f", DIR_LIB, "dir_from") else {
        return;
    };
    let decl_f = "enum Direction { North, East, South, West }\npub fn dir_from(n: integer) -> Direction not null;\n#native \"loft_shared_n_dir_from\"\n";
    run_shared_dispatch(
        &so_f,
        decl_f,
        "fn main() { d = dir_from(1); r = match d { East => 1, _ => 0 }; assert(r == 1, \"dir_from(1) should be East, got tag {r}\") }",
    );
    let _ = fa::remove_dir_all(&tmp_f);
}

/// Data enum (variants carrying fields): `--native` represents it as a `DbRef`
/// (like a struct), both arg and return — no hidden dest.
const SHAPE_LIB: &str = "enum Shape { Circle { r: integer }, Rect { w: integer, h: integer } }\n\
                         pub fn area(s: Shape) -> integer {\n\
                         \x20   match s { Circle { r } => r * r * 3, Rect { w, h } => w * h }\n\
                         }\n\
                         pub fn make_rect(w: integer, h: integer) -> Shape {\n\
                         \x20   Rect { w: w, h: h }\n\
                         }";

/// N2 store-touching: a data `enum` crosses the boundary as a `DbRef`, both as an
/// ARG (`area(Circle{r:2}) == 12`) and a RETURN (`make_rect(3,4)` then `area` → 12),
/// the latter allocated by native code in the shared store.
#[test]
fn dispatches_data_enum_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // area: data-enum arg, scalar return.
    let Some((so_a, tmp_a)) = build_shared_lib_cdylib("loft_n2_denum_a", SHAPE_LIB, "area") else {
        return;
    };
    let decl_a = "enum Shape { Circle { r: integer }, Rect { w: integer, h: integer } }\npub fn area(s: Shape) -> integer not null;\n#native \"loft_shared_n_area\"\n";
    run_shared_dispatch(
        &so_a,
        decl_a,
        "fn main() { s = Circle { r: 2 }; a = area(s); assert(a == 12, \"area(Circle r=2) should be 12, got {a}\") }",
    );
    let _ = fa::remove_dir_all(&tmp_a);

    // make_rect: data-enum RETURN (allocated in the shared store), read via area.
    let Some((so_m, tmp_m)) = build_shared_lib_cdylib("loft_n2_denum_m", SHAPE_LIB, "make_rect")
    else {
        return;
    };
    let decl_m = "enum Shape { Circle { r: integer }, Rect { w: integer, h: integer } }\npub fn make_rect(w: integer, h: integer) -> Shape not null;\n#native \"loft_shared_n_make_rect\"\n";
    run_shared_dispatch(
        &so_m,
        decl_m,
        "fn main() { s = make_rect(3, 4); r = match s { Rect { w, h } => w * h, _ => 0 }; assert(r == 12, \"make_rect(3,4) area should be 12, got {r}\") }",
    );
    let _ = fa::remove_dir_all(&tmp_m);
}

/// A function taking a keyed `sorted` collection (`DbRef` to the container, like
/// a vector — the body walks it through the shared store).
const SORTED_LIB: &str = "struct Item { k: integer not null, v: integer not null }\n\
                          pub fn sum_values(items: sorted<Item[k]>) -> integer {\n\
                          \x20   total = 0;\n\
                          \x20   for it in items { total += it.v; }\n\
                          \x20   total\n\
                          }";

/// N2 store-touching: a keyed `sorted` aggregate crosses the boundary as a
/// `DbRef` (same ABI as a vector/struct).  The native body walks the collection
/// in the shared store.  `sum_values({k:1 v:10, k:2 v:20}) == 30`.  (Uses a
/// hand-written `#native` decl — `generate_interface` does not yet render the
/// `sorted<T[key]>` type name.)
#[test]
fn dispatches_sorted_arg_into_shared_cdylib() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((so, tmp)) = build_shared_lib_cdylib("loft_n2_sorted", SORTED_LIB, "sum_values")
    else {
        return;
    };

    let native_decl = "struct Item { k: integer not null, v: integer not null }\npub fn sum_values(items: sorted<Item[k]>) -> integer not null;\n#native \"loft_shared_n_sum_values\"\n";
    let source = r#"
fn main() {
    s: sorted<Item[k]> = [];
    s += [Item { k: 1, v: 10 }];
    s += [Item { k: 2, v: 20 }];
    a = sum_values(s);
    assert(a == 30, "sum_values should be 30, got {a}")
}
"#;
    run_shared_dispatch(&so, native_decl, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// N2 lean interface: a script drives native dispatch using ONLY the
/// auto-generated interface (`native_lib::generate_interface`) — the library's
/// public type defs + `#native` forward-decls as loft source.  No hand-written
/// struct/enum redefinition and no hand-written `#native` decl: the script adopts
/// the library's exact types (in the library's order, so ids align) and dispatches
/// `make_rect`/`area` round-trip.
#[test]
fn lean_interface_drives_shared_dispatch() {
    use loft::compile::byte_code;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((rlib, deps)) = find_loft_rlib() else {
        return;
    };
    if Command::new("rustc").arg("--version").output().is_err() {
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n2_iface_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();

    // Parse the library (Shape + area + make_rect) and pick both as the export set.
    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(SHAPE_LIB, "lib", false);
    scopes::check(&mut p.data, &mut p.database);
    let area_nr = p.data.def_nr("n_area");
    let make_nr = p.data.def_nr("n_make_rect");
    let shared = loft::native_gate::shared_store_dispatchable(&p.data);
    assert!(shared.contains(&area_nr) && shared.contains(&make_nr));
    let export: std::collections::HashSet<u32> = [area_nr, make_nr].into_iter().collect();

    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // The two generated artifacts: the cdylib and the loft-source interface.
    let interface = loft::native_lib::generate_interface(&p.data, &export);
    println!("--- generated interface ---\n{interface}\n---------------------------");
    let src = loft::native_lib::generate_shared_cdylib_lib_rs(&p.data, &state.database, &export);
    let so = compile_cdylib(&src, "loft_n2_iface", &tmp, &rlib, &deps);

    // The script uses ONLY the generated interface as its native declaration.
    let source = r#"
fn main() {
    s = make_rect(3, 4);
    a = area(s);
    assert(a == 12, "lean-interface dispatch: make_rect then area should be 12, got {a}")
}
"#;
    run_shared_dispatch(&so, &interface, source);
    let _ = fa::remove_dir_all(&tmp);
}

/// N3 core: a NORMAL library function (a body, NO hand-written `#native`) is
/// auto-marked for native dispatch (`native_lib::mark_native_exports`),
/// auto-compiled to a shared cdylib, and dispatched — the script calls it
/// normally, with no `#native` decl anywhere.  This is the in-process shape of
/// what `use <lib>` will do: parse the library into the `Data`, mark its
/// dispatchable functions native, build + load the cdylib, wire the bridge.
#[test]
fn auto_native_marks_and_dispatches_normal_library_fn() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if loft::native_lib::find_loft_rlib().is_none()
        || Command::new("rustc").arg("--version").output().is_err()
    {
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_auto_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();

    // One Data: the library function + the calling script, as `use` would produce.
    // `double` is a NORMAL function — a body, `pub`, and NO `#native` annotation.
    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(
        "pub fn double(x: integer) -> integer { x * 2 }",
        "mylib",
        false,
    );
    p.parse_str(
        "fn main() { r = double(21); assert(r == 42, \"auto-native double(21) should be 42, got {r}\") }",
        "test",
        false,
    );
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(!has_errors, "diagnostics: {:?}", p.diagnostics.lines());
    scopes::check(&mut p.data, &mut p.database);

    // Auto-mark the library's function native (the `use`-time hook).  Before
    // byte_code, so calls route through OpStaticCall.
    let double_nr = p.data.def_nr("n_double");
    let candidates: std::collections::HashSet<u32> = std::iter::once(double_nr).collect();
    let export = loft::native_lib::mark_native_exports(&mut p.data, &candidates);
    assert!(
        export.contains(&double_nr),
        "double should be auto-marked native"
    );
    assert_eq!(
        p.data.def(double_nr).native(),
        "loft_shared_n_double",
        "the bridge symbol must be set"
    );

    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Build the cdylib from the marked export set via the PRODUCTION build path
    // (locates this build's libloft.rlib, generates lib.rs, rustc-compiles), then
    // load + wire + run — exactly the steps `use <lib>` runs after byte_code.
    let so = loft::native_lib::build_shared_cdylib(
        &p.data,
        &state.database,
        &export,
        &tmp,
        "loft_n3_auto",
    )
    .expect("build_shared_cdylib");
    extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    extensions::wire_shared_native_fns(&mut state, &p.data);
    state.execute_argv("main", &p.data, &[]);

    let _ = fa::remove_dir_all(&tmp);
}

/// #307 — BODY-BEARING **text-returning** library fns through the auto-native
/// path.  Unlike the no-body `#native`-decl tests above, a body-bearing def
/// carries `text_return` work-buffer attributes (`RefVar(Text)`), and its call
/// sites push a DbRef per buffer plus typed null sentinels for omitted
/// (defaulted) arguments.  Three protocol seams broke at once:
///   - `compute_sig` rejected the `RefVar(Text)` attr → the bridge never wired
///     → "native function not loaded" stub panic at the first call;
///   - `gen_cdylib_text_dest_call` pushed 0 bytes per `null` arg while popping
///     the full parameter size → frame desync ("Incorrect var ..." panic);
///   - the bridge wrapper substituted a local `String` for the caller's work
///     buffer → every NON-dest call shape (the call nested inside a larger
///     expression) lost its result text.
///
/// One source, three call shapes — assignment (dest-mode), nested-in-concat
/// (non-dest), and defaulted-null arguments.
#[test]
fn auto_native_text_return_shapes() {
    use loft::compile::byte_code;
    use loft::extensions;
    use loft::scopes;
    use loft::state::State;

    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if loft::native_lib::find_loft_rlib().is_none()
        || Command::new("rustc").arg("--version").output().is_err()
    {
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_text_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();

    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(
        "pub fn greet(a: text, b: text, c: text) -> text { a + \":\" + b + \":\" + c }",
        "mylib",
        false,
    );
    p.parse_str(
        "fn main() {\n\
         \x20   r = greet(\"x\", \"y\", \"z\");\n\
         \x20   assert(r == \"x:y:z\", \"assignment (dest-mode) shape, got {r}\");\n\
         \x20   d = greet(\"x\", \"\", \"\");\n\
         \x20   assert(d == \"x::\", \"explicit empty args (defaulted-null lenience removed), got {d}\");\n\
         \x20   n = \"[\" + greet(\"a\", \"b\", \"c\") + \"]\";\n\
         \x20   assert(n == \"[a:b:c]\", \"nested (non-dest) shape, got {n}\");\n\
         }",
        "test",
        false,
    );
    let has_errors = p.diagnostics.lines().iter().any(|l| l.starts_with("Error"));
    assert!(!has_errors, "diagnostics: {:?}", p.diagnostics.lines());
    scopes::check(&mut p.data, &mut p.database);

    let greet_nr = p.data.def_nr("n_greet");
    let candidates: std::collections::HashSet<u32> = std::iter::once(greet_nr).collect();
    let export = loft::native_lib::mark_native_exports(&mut p.data, &candidates);
    assert!(export.contains(&greet_nr), "greet should be auto-marked");

    let mut state = State::new(p.database);
    // Pre-#307 this byte_code call panicked ("Incorrect var ...") on the
    // defaulted-null call's frame accounting.
    byte_code(&mut state, &mut p.data);

    let so = loft::native_lib::build_shared_cdylib(
        &p.data,
        &state.database,
        &export,
        &tmp,
        "loft_n3_text",
    )
    .expect("build_shared_cdylib");
    extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    extensions::wire_shared_native_fns(&mut state, &p.data);
    // Pre-#307 the wiring skipped `greet` (RefVar attr rejected) and this call
    // hit the "native function not loaded" stub.
    state.execute_argv("main", &p.data, &[]);

    let _ = fa::remove_dir_all(&tmp);
}

/// @PLN11 F3 — does a **default-native-MARKED, BODY-BEARING** library function
/// actually dispatch to its compiled cdylib bridge at runtime, or interpret its own
/// loft body (correct output either way — the no-speedup mechanism)?
///
/// A usage sentinel on the bridge (`SHARED_DISPATCH_HITS`) with a positive control:
///   - **Positive control:** a `#native`-declared, *no-body* shared dispatch — it has
///     nothing to interpret, so a passing `vec_sum==60` means the bridge fired.  If
///     the sentinel stays 0 here, the sentinel is broken — distrust its silence below.
///   - **The question:** the SAME function, parsed *with its loft body* and marked
///     native via `mark_exports` (exactly the default-native path), then run.  One
///     axis varied (no-body → body-bearing); the bridge sentinel reports which path
///     the call took.  Observation only (no outcome assert) — it decides the F3 root.
#[test]
fn f3_body_bearing_marked_fn_dispatch_vs_interpret() {
    use loft::state::SHARED_DISPATCH_HITS;
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    // ---- POSITIVE CONTROL: a no-body #native dispatch MUST fire the bridge ----
    let Some((so_pc, tmp_pc)) = build_shared_lib_cdylib("loft_n2_f3pc", VEC_SUM_LIB, "vec_sum")
    else {
        return; // rustc unavailable
    };
    let native_decl = "pub fn vec_sum(data: vector<integer>) -> integer not null;\n\
                       #native \"loft_shared_n_vec_sum\"\n";
    let pc_src = "fn main() { d = [10, 20, 30]; assert(vec_sum(d) == 60, \"pc\") }";
    SHARED_DISPATCH_HITS.store(0, Ordering::Relaxed);
    run_shared_dispatch(&so_pc, native_decl, pc_src);
    let pc_bridge = SHARED_DISPATCH_HITS.load(Ordering::Relaxed);
    let _ = fa::remove_dir_all(&tmp_pc);
    assert!(
        pc_bridge > 0,
        "POSITIVE CONTROL FAILED: the bridge sentinel never moved for a no-body \
         #native dispatch — the sentinel is broken, so its silence below is meaningless"
    );

    // ---- THE QUESTION: same fn, but BODY-BEARING + default-native MARKED ----
    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(VEC_SUM_LIB, "lib", false); // vec_sum WITH its loft body
    p.parse_str(
        "fn main() { d = [10, 20, 30]; assert(vec_sum(d) == 60, \"q\") }",
        "main",
        false,
    );
    assert!(
        !p.diagnostics.lines().iter().any(|l| l.starts_with("Error")),
        "parse: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data, &mut p.database);

    let fn_nr = p.data.def_nr("n_vec_sum");
    let export: HashSet<u32> = std::iter::once(fn_nr).collect();
    let tmp = std::env::temp_dir().join(format!("loft_n2_f3q_{}", std::process::id()));
    let _ = fa::create_dir_all(&tmp);
    let so = match loft::native_lib::build_shared_cdylib(
        &p.data,
        &p.database,
        &export,
        &tmp,
        "loft_auto_f3q",
    ) {
        Ok(so) => so,
        Err(e) => {
            let _ = fa::remove_dir_all(&tmp);
            panic!("build_shared_cdylib failed: {e}");
        }
    };
    loft::native_lib::mark_exports(&mut p.data, &export); // the default-native mark

    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    loft::extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    loft::extensions::wire_shared_native_fns(&mut state, &p.data);

    SHARED_DISPATCH_HITS.store(0, Ordering::Relaxed);
    state.execute_argv("main", &p.data, &[]); // inner assert == positive control the call ran
    let q_bridge = SHARED_DISPATCH_HITS.load(Ordering::Relaxed);
    let _ = fa::remove_dir_all(&tmp);

    eprintln!("F3 SENTINEL  positive-control(no-body) bridge_hits={pc_bridge}");
    eprintln!("F3 SENTINEL  question(body-bearing-marked) bridge_hits={q_bridge}");

    // The liveness guard the arc was missing: parity tests assert OUTPUT (correct
    // whether the call dispatches or interprets its body); this asserts the call
    // actually REACHED the bridge.  A regression that silently reverts default-native
    // to interpret-the-body would pass every parity test but trip here.
    assert!(
        q_bridge > 0,
        "a body-bearing default-native-marked fn must DISPATCH to its cdylib bridge, \
         not interpret its loft body — got bridge_hits={q_bridge} (output was still \
         correct, which is exactly why output-parity tests can't catch this)"
    );
}

/// #294 — the auto-native cdylib stem is the crate name rustc derives from the
/// source-file stem, so it MUST be a valid Rust identifier.  A registry dir
/// carries a dotted version (`glb-0.1.0`); rustc maps `-`→`_` itself but rejects
/// the surviving `.`.  Pins the exact reproducer plus the full invariant: for any
/// directory name the stem is `[A-Za-z0-9_]*` with an alphabetic leading char.
#[test]
fn auto_cdylib_stem_is_a_valid_crate_identifier() {
    use loft::native_lib::auto_cdylib_stem;

    // The exact #294 reproducer: dotted version must not survive into the name.
    assert_eq!(
        auto_cdylib_stem("/home/u/.loft/registry/glb-0.1.0"),
        "loft_auto_glb_0_1_0"
    );

    // A plain (un-versioned) dir name is unchanged apart from the prefix.
    assert_eq!(auto_cdylib_stem("/some/where/datalib"), "loft_auto_datalib");

    // The invariant holds for every shape — multi-dot versions, pre-release
    // suffixes, and other non-identifier punctuation all collapse to `_`.
    for dir in [
        "glb-0.1.0",
        "my.lib-1.2.3-rc.4",
        "weird+name@v2",
        "trailing-slash/",
        "",
    ] {
        let stem = auto_cdylib_stem(dir);
        assert!(
            stem.starts_with("loft_auto_"),
            "stem must keep the alphabetic prefix: {stem:?}"
        );
        let first = stem.chars().next().unwrap();
        assert!(
            first.is_ascii_alphabetic(),
            "a crate name may not start with a digit: {stem:?}"
        );
        assert!(
            stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "every char must be valid in a Rust identifier: {stem:?}"
        );
    }
}

/// #303 — a `text`-returning marked fn must dispatch correctly in EXPRESSION
/// context (`f(7) != f(8)`), not just dest context (`x = f(7)`).
///
/// The fn's local-text-returned shape gives it a `text_return` work-buffer
/// attribute (`&text`).  Before the unified marshallability judgment
/// (`native_gate::classify_bridge_attr`), wire-time `compute_sig` rejected that
/// attribute (`RefVar` → `None`) so the fn never wired and its emitted
/// `OpStaticCall`s hit the panicking "native function not loaded" stub; and
/// the dispatcher's non-dest text return degraded to an empty `Str`, so an
/// expression-context comparison silently evaluated `"" != ""` → `false` —
/// the crawler's `diff_seed` Heisenbug.  This pins both: the call DISPATCHES
/// (sentinel) and both contexts produce the interpreted-identical values
/// (loft-side asserts).
#[test]
fn p303_text_return_marked_fn_expression_and_dest_context() {
    use loft::data::Type;
    use loft::state::SHARED_DISPATCH_HITS;
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    const ENAME_LIB: &str = "pub fn ename(k: integer) -> text {\n\
                             \x20   n = \"amber\";\n\
                             \x20   if k == 7 { n = \"azure\"; }\n\
                             \x20   n\n\
                             }";
    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(ENAME_LIB, "lib", false);
    p.parse_str(
        "fn main() {\n\
         \x20   diff = ename(7) != ename(8);\n\
         \x20   assert(diff, \"expression-context text dispatch must compare real values\");\n\
         \x20   a = ename(7);\n\
         \x20   assert(a == \"azure\", \"dest-context text dispatch\");\n\
         }",
        "main",
        false,
    );
    assert!(
        !p.diagnostics.lines().iter().any(|l| l.starts_with("Error")),
        "parse: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data, &mut p.database);

    let fn_nr = p.data.def_nr("n_ename");
    // Precondition the whole test rests on: the fn carries a text_return
    // work-buffer attribute (`&text`) — the attr kind the old wire path rejected.
    assert!(
        p.data
            .def(fn_nr)
            .attributes()
            .iter()
            .any(|a| matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_)))),
        "precondition: ename must carry a text_return work buffer"
    );

    let export: HashSet<u32> = std::iter::once(fn_nr).collect();
    let tmp = std::env::temp_dir().join(format!("loft_n2_p303_{}", std::process::id()));
    let _ = fa::create_dir_all(&tmp);
    let so = match loft::native_lib::build_shared_cdylib(
        &p.data,
        &p.database,
        &export,
        &tmp,
        "loft_auto_p303",
    ) {
        Ok(so) => so,
        Err(e) => {
            let _ = fa::remove_dir_all(&tmp);
            panic!("build_shared_cdylib failed: {e}");
        }
    };
    loft::native_lib::mark_exports(&mut p.data, &export);

    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    loft::extensions::load_all(&mut state, vec![so.to_string_lossy().into_owned()]);
    loft::extensions::wire_shared_native_fns(&mut state, &p.data);

    SHARED_DISPATCH_HITS.store(0, Ordering::Relaxed);
    state.execute_argv("main", &p.data, &[]); // loft asserts check the values
    let hits = SHARED_DISPATCH_HITS.load(Ordering::Relaxed);
    let _ = fa::remove_dir_all(&tmp);
    assert!(
        hits >= 3,
        "all three ename calls must DISPATCH to the bridge (got bridge_hits={hits}) — \
         a wire-time skip would leave the panicking stub or interpret silently"
    );
}

/// #305 — two modules may export the SAME `pub fn` name (`Data` scopes defs by
/// source; the interpreter resolves module-qualified calls fine), but emitted
/// Rust is one flat namespace: the generated cdylib defined `n_dup_name` and
/// its `loft_shared_n_dup_name` wrapper TWICE -> rustc E0428 -> the library
/// silently fell back to interpreting (and a whole-program `--native` compile
/// failed outright).  Collision members now get a file-hash-disambiguated
/// identifier.  End-to-end over a real on-disk package (module scoping comes
/// from package resolution, which in-process `parse_str` does not model):
/// the auto-cdylib must BUILD (no silent interpret fallback) and both
/// module-qualified calls must dispatch with correct values.
#[test]
fn auto_native_disambiguates_duplicate_fn_names() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if loft::native_lib::find_loft_rlib().is_none()
        || Command::new("rustc").arg("--version").output().is_err()
    {
        return;
    }
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_n3_dup_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let pkg_src = root.join("libs/bundle/src");
    fa::create_dir_all(&pkg_src).unwrap();
    fa::write(
        root.join("libs/bundle/loft.toml"),
        "[package]\nname = \"bundle\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n[library]\nentry = \"src/bundle.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg_src.join("ra.loft"),
        "pub fn dup_name() -> integer { 1 }\n",
    )
    .unwrap();
    fa::write(
        pkg_src.join("rb.loft"),
        "pub fn dup_name() -> integer { 2 }\n",
    )
    .unwrap();
    fa::write(
        pkg_src.join("bundle.loft"),
        "pub use ra::*;\npub use rb::*;\n\npub fn both() -> integer { ra::dup_name() * 10 + rb::dup_name() }\n",
    )
    .unwrap();
    let prog = root.join("prog.loft");
    fa::write(
        &prog,
        "use bundle::*;\n\nfn main() {\n    print(\"both={both()}\\n\");\n}\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(&prog)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stdout}\n{stderr}");
    assert!(
        stdout.contains("both=12"),
        "qualified dup dispatch: {stdout}"
    );
    // The auto-cdylib must have BUILT — pre-#305 it failed E0428 and fell back
    // to interpreting with a "could not compile native" warning.
    assert!(
        !stderr.contains("could not compile native"),
        "cdylib build fell back to interpret (#305 regression):\n{stderr}"
    );

    let _ = fa::remove_dir_all(&root);
}

/// #311 — a vector-returning auto-native call must write its result into the
/// CALLER's pre-allocated hidden destination (forwarded by the dispatcher),
/// not a bridge-local allocation that orphans the caller's record.  The leak
/// was visible as a "stores not freed at program exit" warning — one record
/// per call.  End-to-end over an on-disk package; asserts correct values AND
/// a leak-free exit.
#[test]
fn auto_native_vector_return_uses_caller_dest() {
    let _lock = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if loft::native_lib::find_loft_rlib().is_none()
        || Command::new("rustc").arg("--version").output().is_err()
    {
        return;
    }
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_n3_vdest_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let pkg_src = root.join("libs/vlib/src");
    fa::create_dir_all(&pkg_src).unwrap();
    fa::write(
        root.join("libs/vlib/loft.toml"),
        "[package]\nname = \"vlib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n[library]\nentry = \"src/vlib.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg_src.join("vlib.loft"),
        "pub fn pair(a: text) -> vector<text> { [a, a] }\n",
    )
    .unwrap();
    let prog = root.join("prog.loft");
    fa::write(
        &prog,
        "use vlib::*;\n\nfn main() {\n    v = pair(\"ab\");\n    print(\"[{v[0]}|{v[1]}]\\n\");\n}\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(&prog)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "run failed: {stdout}\n{stderr}");
    assert!(stdout.contains("[ab|ab]"), "vector return value: {stdout}");
    assert!(
        !stderr.contains("could not compile native"),
        "cdylib build fell back to interpret:\n{stderr}"
    );
    assert!(
        !stderr.contains("stores not freed"),
        "caller's hidden dest leaked (#311 regression):\n{stderr}"
    );

    let _ = fa::remove_dir_all(&root);
}
