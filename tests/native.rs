// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native-backend integration tests.
//!
//! These tests compile `.loft` files through the `--native` Rust code generator
//! and run the resulting binaries.  They do **not** acquire `WRAP_LOCK`, so they
//! run concurrently with the interpreter-based `wrap` tests — which is safe
//! because native tests write only to `/tmp/loft_native_*` temp files and never
//! touch the same files as the interpreter tests.

extern crate loft;

use loft::compile::byte_code;
use loft::generation::Output;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
mod common;
use common::cached_default;

/// Process-wide mutex serialising the five high-level native tests
/// (`native_dir`, `native_scripts`, `native_binary_script`,
/// `native_tuple_return_script`, `native_tuple_script`).
///
/// Each test already parallelises internally via `thread::scope` over
/// `available_parallelism()` rustc workers — running two such pools
/// concurrently saturates the CPU twice over, so within ONE process
/// this lock keeps that 2× over-subscription (and the ~140 s→~14 s
/// blowup) away.
///
/// It does NOT, by itself, fix the flaky link failures that once looked
/// like a deps-read race: the real cause was concurrent compiles of the
/// SAME script stem writing the SAME `loft_native_<stem>_bin` output
/// (`50-tuples.loft` is built by three tests), truncating each other's
/// binary mid-link.  Across PROCESSES — nextest runs each test in its
/// own — this in-process lock can't help; correctness there comes from
/// `compile_native_job` compiling to a per-process temp and publishing
/// via atomic `rename(2)`.  See @PLN11 § Discovered follow-ups F1.
fn native_suite_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Files in `tests/docs/` that `native_dir` must not compile.
///
/// Empty, and staying empty is the point: every page the site publishes is a
/// program that runs on both backends, so an entry here is a page whose code a
/// reader cannot trust.  Add one only with the open issue that explains it, and
/// delete it the day that issue closes.
const NATIVE_SKIP: &[&str] = &[];

/// Script files to skip in native mode.
const SCRIPTS_NATIVE_SKIP: &[&str] = &[
    // 135-vector-u8-concat.loft was here for @P316 (`vector<u8>` element read
    // with `?? <int>` mis-compiled); @P316 is fixed, so 135 now runs natively
    // and doubles as the @P316 regression guard.
    //
    // 191-source-dir.loft runs natively as of @PLN9 Phase 1 — the exe-dir anchor
    // (`Stores::source_dir_native` via `current_exe()`) makes `source_dir()`
    // non-empty under `--native`, so it is no longer skipped here.
    //
];

/// Locate `libloft.rlib` and its sibling deps directory for standalone `rustc` compilation.
///
/// Searches only the deps directory of the currently running test binary so that
/// the rlib always matches the features compiled into this test.  The old approach
/// of scanning both profiles and picking by mtime caused S33 in CI: a later
/// `cargo build --no-default-features` produced a newer no-features rlib in the
/// other profile's deps/, shadowing the full-features rlib and leaving png/random
/// functions as stubs that silently return wrong values.
fn find_loft_rlib() -> Option<(PathBuf, PathBuf)> {
    // The test binary lives at target/{profile}/deps/{test_binary}.
    // Its parent is the deps/ directory that holds the rlib built with the same features.
    let deps = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;

    // Find the most recently modified loft rlib in this profile's deps/.
    // Accept both "libloft-HASH.rlib" (cargo test profile) and "libloft.rlib"
    // (produced when building lib+test together).  Both live in the same
    // profile-specific deps/ directory, so there is no cross-profile shadowing
    // (the S33 risk only arose when scanning multiple profile directories).
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

/// Collect additional `--extern name=path` arguments for optional feature dependencies.
///
/// Collect additional `--extern name=path` arguments for optional feature dependencies.
///
/// When `rustc` compiles generated `.rs` files standalone, it only knows about crates
/// explicitly declared via `--extern`.  Optional deps like `rand_core` and `rand_pcg`
/// are available in the same deps/ directory as `libloft.rlib` but must be declared
/// explicitly (S31).  This function scans deps/ and returns ALL non-loft rlibs as
/// `(crate_name, rlib_path)` pairs.  All versions of each crate are included so that
/// rustc can select the hash that matches what `libloft` was compiled against.
fn collect_extra_externs(deps_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(deps_dir) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("lib") || !name.ends_with(".rlib") || name.starts_with("libloft") {
            continue;
        }
        // libFOO-HASH.rlib → crate name FOO (hyphens → underscores)
        let without_lib = &name[3..];
        let without_rlib = without_lib.trim_end_matches(".rlib");
        let crate_name = if let Some(pos) = without_rlib.rfind('-') {
            without_rlib[..pos].replace('-', "_")
        } else {
            without_rlib.replace('-', "_")
        };
        result.push((crate_name, entry.path()));
    }
    result
}

/// On Windows MSVC, locate build-script output directories for native import libraries.
///
/// When linking against `libloft.rlib` with standalone `rustc`, crates like `windows-sys`
/// that emit native import libraries via their build scripts (e.g. `windows.0.48.5.lib`)
/// are not automatically found.  Cargo normally passes the build-script output dirs as
/// `-L native=…` linker arguments; we replicate that here.
///
/// Strategy: add every `out/` subdirectory of `target/{profile}/build/` as a `-L` path,
/// plus each of their immediate subdirectories.  Some crates (e.g. `windows-targets`) place
/// import libraries in a platform-specific subdirectory such as `out/x86_64-pc-windows-msvc/`
/// rather than directly in `out/`.  Adding both levels covers all known layouts.
fn find_native_lib_dirs(rlib_info: &Option<(PathBuf, PathBuf)>) -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        let _ = rlib_info;
        Vec::new()
    }
    #[cfg(windows)]
    {
        let Some((rlib, _)) = rlib_info else {
            return Vec::new();
        };
        // rlib is at target/{profile}/libloft.rlib or target/{profile}/deps/libloft-*.rlib.
        // Walk up to find the profile directory (release/ or debug/).
        let profile_dir = rlib.parent().and_then(|p| {
            if p.file_name().map(|n| n == "deps").unwrap_or(false) {
                p.parent()
            } else {
                Some(p)
            }
        });
        let Some(profile_dir) = profile_dir else {
            return Vec::new();
        };
        let build_dir = profile_dir.join("build");
        let Ok(entries) = std::fs::read_dir(&build_dir) else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let build_entry = entry.path();

            // Add out/ and its immediate subdirs (for libs generated into OUT_DIR).
            let out = build_entry.join("out");
            if out.is_dir() {
                dirs.push(out.clone());
                if let Ok(subdirs) = std::fs::read_dir(&out) {
                    for sub in subdirs.filter_map(|e| e.ok()) {
                        if sub.path().is_dir() {
                            dirs.push(sub.path());
                        }
                    }
                }
            }

            // Read the build-script output file for `cargo:rustc-link-search` directives.
            // Crates like `windows_x86_64_msvc` ship `windows.0.48.5.lib` inside their
            // source package (cargo registry) and emit
            //   cargo:rustc-link-search=<CARGO_MANIFEST_DIR>
            // rather than writing the file to OUT_DIR.  Cargo caches these directives in
            // `target/{profile}/build/{crate}-{hash}/output`.  Reading them here replicates
            // exactly what cargo passes to the linker.
            let output_file = build_entry.join("output");
            if let Ok(content) = std::fs::read_to_string(&output_file) {
                for line in content.lines() {
                    let path_str = line
                        .strip_prefix("cargo:rustc-link-search=native=")
                        .or_else(|| line.strip_prefix("cargo:rustc-link-search="));
                    if let Some(path_str) = path_str {
                        let p = PathBuf::from(path_str);
                        if p.is_dir() && !dirs.contains(&p) {
                            dirs.push(p);
                        }
                    }
                }
            }
        }
        dirs
    }
}

/// Paths for one native compilation job.
struct NativeJob {
    stem: String,
    tmp_rs: PathBuf,
    binary: PathBuf,
    /// Sidecar file that stores the cache key written at compile time.
    /// Path: `{binary}.key`.  Content: hex-encoded 64-bit FNV-1a hash of the
    /// `.rs` source bytes concatenated with the rlib identity bytes.
    key_file: PathBuf,
}

/// FNV-1a 64-bit hash — deterministic, no external deps.
fn fnv64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Build the cache key from the current `.rs` content and rlib identity.
///
/// The key captures both what was compiled (`.rs` bytes) and what it was
/// linked against (rlib path + CONTENT hash).  If either changes the key
/// changes and the binary is recompiled.
///
/// BUILD2: keyed on rlib bytes, not mtime.  `actions/cache` persists `target/`
/// but every CI run reruns `cargo build --release --lib`, which rewrites
/// `libloft.rlib` with a fresh mtime even on a no-op rebuild — an mtime fold
/// then misses and recompiles every native fixture.  rustc's rlib output is
/// byte-deterministic for unchanged sources, so a content hash is stable across
/// the no-op rebuild (warm-cache hit) while still invalidating when the binary
/// actually changes (different bytes → different hash).  The rlib is read once
/// per process via `rlib_content_hash`, not once per fixture.
fn cache_key(rs_content: &[u8], rlib_info: &Option<(PathBuf, PathBuf)>) -> u64 {
    let mut key = fnv64(rs_content);
    if let Some((rlib, _)) = rlib_info {
        key ^= fnv64(rlib.to_string_lossy().as_bytes());
        key ^= rlib_content_hash(rlib);
    }
    key
}

/// FNV-1a hash of a file's bytes, memoised per path (the rlib is ~14MB and the
/// same within a run).  Missing file → 0, matching the old no-op-on-missing.
fn rlib_content_hash(path: &Path) -> u64 {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(&h) = guard.get(path) {
        return h;
    }
    let h = std::fs::read(path).map(|b| fnv64(&b)).unwrap_or(0);
    guard.insert(path.to_path_buf(), h);
    h
}

/// Phase 1 — parse the `.loft` file and generate its Rust source.
///
/// The generated `.rs` is written only when its content changes, so that the binary
/// modification-time cache in Phase 2 is not unnecessarily invalidated.
///
/// Fails the test if the loft parse or scope-check step produces diagnostics.
fn prepare_native_test(entry: &Path) -> std::io::Result<NativeJob> {
    let stem = entry
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace('-', "_");
    println!("native {entry:?}");

    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    // Honour `// @ARGS: --lib <dir>` lines at the top of the test file
    // so scripts using `use foo::*` can locate fixtures alongside
    // wrap.rs's loft_suite (e.g. tests/lib/importlib.loft for
    // 88-imports.loft).  Only `--lib <dir>` is recognised; other
    // CLI-side flags are ignored at this layer.
    if let Ok(src) = std::fs::read_to_string(entry) {
        for line in src.lines().take(20) {
            if let Some(args) = line.trim().strip_prefix("// @ARGS:") {
                let mut tokens = args.split_whitespace();
                while let Some(tok) = tokens.next() {
                    if tok == "--lib"
                        && let Some(dir) = tokens.next()
                    {
                        p.lib_dirs.push(dir.to_string());
                    }
                }
            }
        }
    }
    let start_def = p.data.definitions();
    p.parse(&entry.to_string_lossy(), false);
    // Two sources may each declare a function of the same name, and emitted Rust is one
    // flat namespace.  `namespace_colliding_native_fns` is what settles that — the lowest
    // source keeps the bare name, the rest become `n_s<N>_<name>` — and its own contract is
    // that it runs after the two-pass parse and BEFORE a native emit.  Every `--native`
    // path in `main.rs` calls it; this harness did not, so it emitted from a `Data` the
    // compiler would never hand the generator: a module's own `main` stayed `n_main` beside
    // the entry's, `duplicate_fn_names` saw a duplicate that no real build has, and the
    // entry was emitted under a disambiguated ident while the template called the bare
    // name (loft#1351).
    p.data.namespace_colliding_native_fns();
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        return Err(Error::from(std::io::ErrorKind::InvalidData));
    }
    scopes::check(&mut p.data, &mut p.database);
    // Two sources may each declare a function of the same name, and emitted Rust is one
    // flat namespace.  `namespace_colliding_native_fns` is what settles that — the lowest
    // source keeps the bare name, the rest become `n_s<N>_<name>` — and its own contract is
    // that it runs after the two-pass parse and BEFORE a native emit.  Every `--native`
    // path in `main.rs` calls it; this harness did not, so it emitted from a `Data` the
    // compiler would never hand the generator: a module's own `main` stayed `n_main` beside
    // the entry's, `duplicate_fn_names` saw a duplicate that no real build has, and the
    // entry was emitted under a disambiguated ident while the template called the bare
    // name (loft#1351).
    p.data.namespace_colliding_native_fns();
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    // The ENTRY FILE's `main`, not any `main` in the table.  `def_nr` is a global lookup
    // and `use` parses a module's definitions into the same table, so a library that
    // happens to declare `fn main()` could win it — and the generated Rust then called an
    // `n_main` that was never emitted (`cannot find function n_main in this scope`,
    // loft#1351).  `MAIN_SOURCE` is the entry; `2..` are the modules it imported.
    let main_nr = (start_def..end_def)
        .find(|d| {
            let def = p.data.def(*d);
            def.name == "n_main" && def.source() == loft::data::MAIN_SOURCE
        })
        .unwrap_or(end_def);
    let has_main = main_nr < end_def;

    // Collect zero-parameter user functions as test entry points.
    let mut test_fns: Vec<(u32, String)> = Vec::new();
    for d_nr in start_def..end_def {
        let def = p.data.def(d_nr);
        // Same narrowing, same reason: an imported module's zero-parameter void function
        // is not this file's test.  `is_corpus_entry_point` excludes the STDLIB by path
        // but says nothing about a `use`d library.
        if def.source() != loft::data::MAIN_SOURCE {
            continue;
        }
        // `Definition::is_corpus_entry_point` is the ONE answer, shared with
        // `tests/wrap.rs`.  This side used to ask a WIDER question — no return filter — so
        // every zero-parameter value-returning function in a main-less corpus file ran here
        // and nowhere else: 165 of them across 66 files, each discarding the store it
        // answers.  A differential whose two halves run different code cannot report a
        // divergence, which is the whole reason the corpus is run twice (loft#1293).
        if !def.is_corpus_entry_point() {
            continue;
        }
        test_fns.push((d_nr, def.name.clone()));
    }

    let entry_defs: Vec<u32> = if has_main {
        vec![main_nr]
    } else {
        test_fns.iter().map(|(d, _)| *d).collect()
    };

    // Generate Rust source into an in-memory buffer first.
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut out = Output::new(&p.data, &state.database);
        out.output_native_reachable(&mut buf, start_def, end_def, &entry_defs)?;
    }

    // A file the GENERATOR itself refused cannot be a native job.  `compile_error!` is
    // what loft emits for a shape `--native` cannot express — a `#native` fn with no
    // registered implementation, a refused coroutine yield type — and the macro fails the
    // build wherever it is expanded, so reachable-but-never-called is not a rescue.
    //
    // This is the honest form of the drop that `@EXPECT_FAIL` used to stand in for.  The
    // annotation says a function is expected to FAIL; it never said the file could not be
    // COMPILED, and reading it as if it did is what cost every sibling its coverage
    // (loft#1311).  Keyed on the refusal itself, this also covers a file that carries no
    // annotation at all.
    if String::from_utf8_lossy(&buf).contains("compile_error!") {
        return Err(Error::other(
            "native codegen refused this file (compile_error! in the generated Rust)",
        ));
    }

    // For test-style files without fn main(), generate a main() that calls
    // each test function so the native binary is a valid executable.
    // Skip functions marked with @EXPECT_FAIL in the source.
    if !has_main && !test_fns.is_empty() {
        use std::io::Write;
        let src = std::fs::read_to_string(entry).unwrap_or_default();
        // The SAME parser the interpreter runner reads the annotation with, so the two
        // cannot disagree about which functions a file excuses.  A second parser here
        // keyed on words-on-the-line could not see the documented
        // `// @EXPECT_FAIL: <reason>` form — the token carries the colon — and came back
        // empty for every file that used it (loft#1311).
        let (expect_fail_fns, _file_level) = common::expect_fail_fns(&src);
        // P199 — wrap Stores in UnsafeCell for the new ABI; the work
        // buffers (`stores.null_named(...)`) need a temporary `&mut Stores`
        // derived from the cell.
        writeln!(buf, "\nfn main() {{")?;
        writeln!(
            buf,
            "    let cell = std::cell::UnsafeCell::new(Stores::new());"
        )?;
        writeln!(
            buf,
            "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        writeln!(buf, "    init(&cell);")?;
        for (d_nr, name) in &test_fns {
            let user_name = name.strip_prefix("n_").unwrap_or(name);
            // Exact: the parser yields the name off the `fn` line, so a substring test
            // would also skip a sibling whose name merely contains the excused one.
            if expect_fail_fns.contains(user_name) {
                writeln!(buf, "    // skipped (EXPECT_FAIL): {name}")?;
            } else {
                // Generate work-buffer locals for hidden __work_* / __ref_* parameters
                // that text_return adds to text-returning functions.
                let def = p.data.def(*d_nr);
                let mut work_args = Vec::new();
                for (i, attr) in def.attributes.iter().enumerate() {
                    if attr.name.starts_with("__work_") {
                        let wname = format!("_w_{user_name}_{i}");
                        writeln!(buf, "    let mut {wname} = String::new();")?;
                        work_args.push(format!("&mut {wname}"));
                    } else if attr.name.starts_with("__ref_") {
                        let wname = format!("_r_{user_name}_{i}");
                        writeln!(buf, "    let mut {wname} = stores.null_named(\"{wname}\");")?;
                        work_args.push(wname.to_string());
                    }
                }
                if work_args.is_empty() {
                    writeln!(buf, "    {name}(&cell);")?;
                } else {
                    writeln!(buf, "    {name}(&cell, {});", work_args.join(", "))?;
                }
            }
        }
        writeln!(buf, "}}")?;
    }

    // Only write the .rs file when the content has changed.  This keeps the file's
    // content stable across runs where the loft source hasn't changed, which
    // means cache_key() produces the same hash and compile_native_job stays cached.
    // scratch_dir honours LOFT_TMPDIR so the whole native run can be kept off a
    // small /tmp tmpfs; all of these must agree on the same directory.
    let scratch = loft::platform::scratch_dir();
    let tmp_rs = scratch.join(format!("loft_native_{stem}.rs"));
    let existing = std::fs::read(&tmp_rs).unwrap_or_default();
    if existing != buf {
        // Atomic publish: write to a per-process temp then rename into place,
        // so a concurrent process (nextest runs each test in its own process)
        // compiling the same stem never reads a half-written source.  See the
        // shared-output collision note in `compile_native_job`.
        let tmp = scratch.join(format!("loft_native_{stem}_{}.rs.tmp", std::process::id()));
        std::fs::write(&tmp, &buf)?;
        std::fs::rename(&tmp, &tmp_rs)?;
    }

    let binary = scratch.join(format!("loft_native_{stem}_bin"));
    let key_file = scratch.join(format!("loft_native_{stem}_bin.key"));
    Ok(NativeJob {
        stem,
        tmp_rs,
        binary,
        key_file,
    })
}

/// Return true if the cached binary is still valid for the current `.rs` content
/// and rlib.  Uses a content-hash sidecar (`{binary}.key`) written at compile
/// time — immune to clock skew and cross-machine binary copies.
fn binary_cache_valid(job: &NativeJob, rlib_info: &Option<(PathBuf, PathBuf)>) -> bool {
    // Binary must exist.
    if !job.binary.exists() {
        return false;
    }
    // Read the stored key from the sidecar.
    let stored = match std::fs::read_to_string(&job.key_file) {
        Ok(s) => s.trim().to_string(),
        Err(_) => return false,
    };
    // Recompute the key from the current .rs content and rlib.
    let rs_content = match std::fs::read(&job.tmp_rs) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let current_key = cache_key(&rs_content, rlib_info);
    stored == format!("{current_key:016x}")
}

/// Phase 2 — compile the generated `.rs` file to a native binary with `rustc`.
///
/// Skips compilation when `binary_cache_valid` is true (binary is already up to date).
/// The binary is kept on disk after use so that subsequent runs can hit the cache.
///
/// Returns `Ok(true)` when a valid binary is available, `Ok(false)` when `rustc` is
/// not in PATH (caller should skip the run phase), and `Err` on a real compile failure.
fn compile_native_job(
    job: &NativeJob,
    rlib_info: &Option<(PathBuf, PathBuf)>,
) -> std::io::Result<bool> {
    if binary_cache_valid(job, rlib_info) {
        println!("  cached  {}", job.stem);
        // BUILD2: bump the binary's mtime on a cache HIT so its timestamp
        // tracks last-USE, not last-compile.  A long-lived cache entry is
        // hit (not recompiled) run after run, so without this its mtime
        // would freeze at first-compile time and an age-based reaper would
        // delete exactly the entries the cache most wants to keep warm.
        let now = std::time::SystemTime::now();
        let _ = std::fs::File::open(&job.binary).and_then(|f| f.set_modified(now));
        let _ = std::fs::File::open(&job.key_file).and_then(|f| f.set_modified(now));
        return Ok(true);
    }
    // Preflight (Layer 2): never start a compile that could overflow a
    // RAM-backed tmpfs and exhaust memory.  Reclaims loft's own stale
    // artefacts first; skips (not fails) the test if space is still low.
    let scratch = loft::platform::scratch_dir();
    if !loft::platform::native_compile_space_ok(&scratch) {
        println!(
            "  SKIP {} — low temp space in {} (set LOFT_TMPFS_MIN_FREE_MB to tune)",
            job.stem,
            scratch.display()
        );
        return Ok(false);
    }
    // nextest runs each test in its own process, so the native tests that share
    // a stem — `native_tuple_script`, `native_tuple_return_script`, and
    // `native_scripts` all compile `50-tuples.loft` — would otherwise have
    // their rustc/lld processes write the SAME `loft_native_<stem>_bin` output
    // concurrently, truncating each other's binary mid-link (observed as a
    // SIGBUS in `rust-lld` / `linking with cc failed`).  The in-process
    // `native_suite_lock()` only serialises them WITHIN one process.  Compile
    // to a per-process temp and publish atomically (rename) below, so concurrent
    // processes never share a mutable output file — keeping full parallelism
    // without a nextest serial group.
    let pid = std::process::id();
    let binary_tmp = scratch.join(format!("loft_native_{}_{pid}_bin", job.stem));
    // PLAN49 follow-up — build the rustc args into a file passed via
    // `rustc @argfile` instead of as a long command line.  Windows
    // `CreateProcessW` enforces a 32 KB command-line limit; on CI runners
    // the rustc `--extern <crate>=<rlib-path>` list + `-L` search paths
    // from `find_native_lib_dirs` (windows-targets build-script outputs)
    // routinely approaches that limit, and a runner-image bump on
    // 2026-05-29 pushed it over, killing every native test with
    // `Os { code: 206, kind: InvalidFilename }`.  The argfile pattern
    // is cross-platform (Linux + macOS happily accept it too) and
    // immune to cmdline length.
    let mut args: Vec<String> = vec![
        "--edition=2024".to_string(),
        "-C".to_string(),
        "debuginfo=0".to_string(),
        "-C".to_string(),
        "opt-level=0".to_string(),
    ];
    // Layer 1: strip the linked binary (~36MB → ~1MB; the bulk is debug info
    // from libloft.rlib + std, useless to a run-and-check test).  Opt out with
    // LOFT_NATIVE_KEEP_SYMBOLS=1 when debugging a native crash (the generated
    // .rs is always kept for recompilation).
    if loft::platform::native_strip_symbols() {
        args.push("-C".to_string());
        args.push("strip=symbols".to_string());
    }
    // LOFT_CRANELIFT=1 — use the Cranelift codegen backend for much faster compilation.
    // Requires a nightly toolchain with `rustup component add rustc-codegen-cranelift-preview`.
    if std::env::var_os("LOFT_CRANELIFT").is_some() {
        args.push("-Z".to_string());
        args.push("codegen-backend=cranelift".to_string());
    }
    args.push("-o".to_string());
    args.push(binary_tmp.display().to_string());
    args.push(job.tmp_rs.display().to_string());
    if let Some((rlib, deps_dir)) = rlib_info {
        args.push("--extern".to_string());
        args.push(format!("loft={}", rlib.display()));
        args.push("-L".to_string());
        args.push(deps_dir.display().to_string());
        // S31: pass --extern for optional feature deps (rand_core, rand_pcg, etc.) so that
        // generated code using `random` or `png` features compiles without E0433 errors.
        for (crate_name, rlib_path) in collect_extra_externs(deps_dir) {
            args.push("--extern".to_string());
            args.push(format!("{crate_name}={}", rlib_path.display()));
        }
    }
    // On Windows MSVC, build-script output dirs holding native import libs (e.g.
    // `windows.0.48.5.lib` from `windows-sys`) must be passed explicitly to standalone
    // rustc — cargo adds them automatically via `cargo:rustc-link-search`, but we don't.
    for dir in find_native_lib_dirs(rlib_info) {
        args.push("-L".to_string());
        args.push(dir.display().to_string());
    }
    // Write one arg per line.  rustc's argfile parser is whitespace-
    // separated; newline-separated is a strict subset and is what
    // every other tool (clang, gcc) accepts too.  Paths containing
    // whitespace get quoted defensively.
    let argfile_path = scratch.join(format!("loft_native_{}_{pid}_args.txt", job.stem));
    let argfile_contents = args
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
    std::fs::write(&argfile_path, argfile_contents)?;
    let compile_out = match std::process::Command::new("rustc")
        .arg(format!("@{}", argfile_path.display()))
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  rustc not found — skipping native test for {}", job.stem);
            let _ = std::fs::remove_file(&argfile_path);
            return Ok(false);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&argfile_path);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&argfile_path);
    if !compile_out.status.success() {
        let stderr = String::from_utf8_lossy(&compile_out.stderr);
        eprintln!("rustc failed for {}:\n{stderr}", job.stem);
        let _ = std::fs::remove_file(&binary_tmp);
        let _ = std::fs::remove_file(&job.binary);
        let _ = std::fs::remove_file(&job.key_file);
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    // Publish the freshly-linked binary atomically: rename the per-process temp
    // over the shared cache path.  rename(2) swaps the directory entry, so a
    // concurrent process executing or linking the old binary keeps its inode
    // (no in-place truncation → no SIGBUS) and the cache path is always a
    // complete binary.
    std::fs::rename(&binary_tmp, &job.binary)?;
    // Write the cache key so future runs can skip recompilation when nothing
    // changed — also via temp + rename so a concurrent `binary_cache_valid`
    // reader never sees a half-written key.
    let rs_content = std::fs::read(&job.tmp_rs).unwrap_or_default();
    let key = cache_key(&rs_content, rlib_info);
    let key_tmp = scratch.join(format!("loft_native_{}_{pid}_bin.key.tmp", job.stem));
    if std::fs::write(&key_tmp, format!("{key:016x}")).is_ok() {
        let _ = std::fs::rename(&key_tmp, &job.key_file);
    }
    Ok(true)
}

/// Phase 3 — run a compiled native binary and check its exit status.
///
/// The binary is kept on disk after running so it can be reused as a compilation
/// cache on the next invocation (see `binary_cache_valid`).
fn run_native_job(job: &NativeJob) -> std::io::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let run_status = std::process::Command::new(&job.binary)
        .current_dir(&cwd)
        .status()?;
    if !run_status.success() {
        eprintln!(
            "native binary failed for {} (exit {:?})",
            job.stem,
            run_status.code()
        );
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    Ok(())
}

/// Compile in parallel, then run in parallel.
fn run_native_jobs(
    jobs: Vec<NativeJob>,
    rlib_info: Option<(PathBuf, PathBuf)>,
) -> std::io::Result<()> {
    // Layer 3: scale worker count to temp-fs headroom.  On a roomy disk this is
    // just min(cpus, jobs); on a tight RAM-backed tmpfs it clamps down so N
    // concurrent compiles can't exhaust memory and hang the machine.  Each
    // in-flight compile peaks at ~1.2GB of temp (rustc intermediates dominate;
    // the stripped output binary is ~1MB), so reserve that per worker.
    let cpu_max = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    const PER_WORKER_TMP: u64 = 1280 * 1024 * 1024;
    let concurrency = loft::platform::native_worker_count(
        cpu_max,
        jobs.len(),
        &loft::platform::scratch_dir(),
        PER_WORKER_TMP,
    );
    let rlib_ref = &rlib_info;

    // Phase 2: compile all jobs in parallel chunks.
    let mut compiled: Vec<bool> = Vec::with_capacity(jobs.len());
    let mut first_err: Option<std::io::Error> = None;
    for chunk in jobs.chunks(concurrency) {
        let chunk_results: Vec<std::io::Result<bool>> = std::thread::scope(|s| {
            chunk
                .iter()
                .map(|job| s.spawn(|| compile_native_job(job, rlib_ref)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(Error::from(std::io::ErrorKind::Other)))
                })
                .collect()
        });
        for r in chunk_results {
            match r {
                Ok(b) => compiled.push(b),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    compiled.push(false);
                }
            }
        }
    }
    let compile_fail = compiled.iter().filter(|ok| !**ok).count();

    // Phase 3: run all compiled binaries in parallel.
    let ready: Vec<&NativeJob> = jobs
        .iter()
        .zip(compiled.iter())
        .filter(|(_, ok)| **ok)
        .map(|(job, _)| job)
        .collect();
    let compile_ok = ready.len();
    let run_errors: Vec<String> = std::thread::scope(|s| {
        ready
            .iter()
            .map(|job| s.spawn(|| run_native_job(job)))
            .collect::<Vec<_>>()
            .into_iter()
            .zip(ready.iter())
            .filter_map(|(h, job)| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::from(std::io::ErrorKind::Other)))
                    .err()
                    .map(|_| job.stem.clone())
            })
            .collect()
    });
    let run_ok = compile_ok - run_errors.len();
    println!(
        "\nnative result: {run_ok} passed, {} compile failed, {} run failed; {} total",
        compile_fail,
        run_errors.len(),
        jobs.len()
    );
    if !run_errors.is_empty() {
        println!("  run failures: {}", run_errors.join(", "));
    }
    // Fail if any test failed to compile or run.
    if compile_fail > 0 || !run_errors.is_empty() {
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    Ok(())
}

/// Build and run one example through the real `loft --native` binary, the way a
/// user does, and require exit 0.
///
/// This file's own emit path builds a **self-contained crate**: it generates the
/// `.rs` itself and hands rustc loft's own rlib and nothing else. An example that
/// imports a library therefore does not link — `use random` leaves
/// `can't find crate for loft_random` (E0463) — because the flags that would
/// resolve it are missing: the package's `-L native=<build cache>`,
/// `-l dylib=loft_<pkg>` and the two rpaths, plus the C-ABI codegen switch
/// `Output::native_cabi` that decides which call form is emitted for a library
/// call in the first place.
///
/// Those flags are not a constant. They are derived per package by
/// `src/native_utils.rs`, which is `pub(crate)` — an integration test cannot call
/// it, and re-deriving them here would be a SECOND copy of package resolution,
/// free to drift from the one the `loft` binary actually uses. Delegating keeps
/// one home for that logic and tests the command a user types.
///
/// The trade is real and is why this is not the default path: no shared binary
/// cache, no cross-file rustc parallelism, and a cdylib rebuild when the
/// library's artefact is stale. It is worth it only for the files the emit path
/// cannot build at all.
///
/// Returns `Ok(false)` when `rustc` is absent, matching `compile_native_job` — and the CALLER
/// must treat that as a failure, as `run_native_jobs` does, or the value matches while the
/// consequence does not.
fn run_via_loft_binary(entry: &Path) -> std::io::Result<bool> {
    if std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .is_err()
    {
        println!(
            "  rustc not found — skipping native test for {}",
            entry.display()
        );
        return Ok(false);
    }
    // Bounded, because `--native` shells out to rustc and cargo, either of which
    // can hang; the rest of this file inherits the suite watchdog instead.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(entry)
        .env("LOFT_TIMEOUT", "300")
        .output()?;
    if !out.status.success() {
        eprintln!(
            "`loft --native {}` failed (exit {:?}):\n{}{}",
            entry.display(),
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    println!("  delegated  {}", entry.display());
    Ok(true)
}

/// Compile and run every `.loft` file in `tests/docs/` through the native Rust
/// backend (`--native` mode), skipping files listed in `NATIVE_SKIP`.
///
/// Runs concurrently with interpreter-based wrap tests (no WRAP_LOCK).
/// FAILS if `rustc` is not in PATH: a file that could not be built lands in `compile_fail`,
/// and a suite that reported a pass for tests it never compiled would be worse than a red
/// one.  A cache HIT is not that case — it runs the binary it already has.
#[test]
fn native_dir() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/docs")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    let rlib_info = find_loft_rlib();
    let mut jobs = Vec::new();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if NATIVE_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (native skip list — see NATIVE_SKIP)");
            continue;
        }
        jobs.push(prepare_native_test(&entry)?);
    }
    run_native_jobs(jobs, rlib_info)
}

/// Compile and run every extracted feature example in `tests/docs/features/`
/// through the native backend — the native half of @PLN92 strand-3's
/// "example-must-run" guard (@I81).  These files are generated from
/// `loft-lang/features` issues by `tools/features/gen.loft`; only complete-program
/// examples land here (library / syntax fragments are mirrored but not tested).
/// Silent no-op if the directory is absent; skips silently if `rustc` is absent.
#[test]
fn native_features() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut files: Vec<PathBuf> = match std::fs::read_dir("tests/docs/features") {
        Ok(rd) => rd
            .filter_map(|f| f.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    let rlib_info = find_loft_rlib();
    let mut jobs = Vec::new();
    let mut delegated: Vec<PathBuf> = Vec::new();
    for entry in files {
        // An example that imports a library is built by the `loft` binary rather than
        // by this file's emit path, which cannot link one — see `run_via_loft_binary`.
        // Selected by the `use` RULE, not by filename, so the next library example is
        // covered without an edit here.
        let imports_library = std::fs::read_to_string(&entry)
            .map(|src| src.lines().any(|l| l.trim_start().starts_with("use ")))
            .unwrap_or(false);
        if imports_library {
            delegated.push(entry);
            continue;
        }
        jobs.push(prepare_native_test(&entry)?);
    }
    // Both halves run before either verdict is returned, so a failure in one does not
    // hide a failure in the other.
    let emitted = run_native_jobs(jobs, rlib_info);
    let mut failed: Vec<String> = Vec::new();
    for entry in &delegated {
        // `Ok(false)` — no rustc, so the example was never built — counts as a FAILURE, the
        // same as it does on the emit half, where it lands in `compile_fail`.  Testing only
        // `.is_err()` let the two halves read one return value two ways: a missing toolchain
        // failed the emitted examples and was ignored for the delegated ones, so this half
        // reported a pass having run nothing.  These are the examples that IMPORT a library,
        // which is the case least covered elsewhere and the one worth failing loudly for.
        match run_via_loft_binary(entry) {
            Ok(true) => {}
            Ok(false) | Err(_) => failed.push(entry.display().to_string()),
        }
    }
    if !failed.is_empty() {
        eprintln!("delegated native failures: {}", failed.join(", "));
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    emitted
}

/// Compile and run every `.loft` file in `tests/scripts/` through the native Rust
/// backend (`--native` mode), skipping files listed in `SCRIPTS_NATIVE_SKIP`.
///
/// Runs concurrently with interpreter-based wrap tests (no WRAP_LOCK).
/// FAILS if `rustc` is not in PATH: a file that could not be built lands in `compile_fail`,
/// and a suite that reported a pass for tests it never compiled would be worse than a red
/// one.  A cache HIT is not that case — it runs the binary it already has.
// @speed 6.1
#[test]
fn native_scripts() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/scripts")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    let rlib_info = find_loft_rlib();
    let mut jobs = Vec::new();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if SCRIPTS_NATIVE_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (scripts native skip list — see SCRIPTS_NATIVE_SKIP)");
            continue;
        }
        // Skip files with intentional compile errors or expected failures.
        // Native mode compiles the whole file into one binary and can't
        // tolerate per-function panics like the interpreter runner can.
        //
        // Read the annotation the way `wrap` reads it (`common::expect_tag`), not with a
        // `contains` over the whole source: a file that merely NAMES the tag in prose —
        // "this file used to be an @EXPECT_ERROR case" — declares nothing, and skipping
        // on the mention silently dropped five scripts from this suite, including
        // `93-vector-advanced.loft` and its forty-nine assertions.
        if let Ok(src) = std::fs::read_to_string(&entry) {
            if common::declares_expect_error(&src) {
                println!("skip {entry:?} (has @EXPECT_ERROR)");
                continue;
            }
            // Only a FILE-LEVEL `@EXPECT_FAIL` drops the file.  A fn-level one names a
            // single function, and the documented contract is that its siblings still
            // must pass — so dropping the whole file cost every sibling its native
            // coverage, silently (loft#1311).  The fn itself is skipped in
            // `prepare_native_test`, which reads the same parser.
            if common::expect_fail_fns(&src).1 {
                println!("skip {entry:?} (has file-level @EXPECT_FAIL)");
                continue;
            }
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_native_test(&entry)))
        {
            Ok(Ok(job)) => jobs.push(job),
            Ok(Err(e)) => println!("skip {entry:?} (prepare error: {e})"),
            Err(_) => println!("skip {entry:?} (codegen panic — native codegen bug)"),
        }
    }
    run_native_jobs(jobs, rlib_info)
}

/// N8a: native code generation for tuple types.
///
/// Runs `tests/scripts/50-tuples.loft` through the native Rust backend end-to-end.
/// Ignored until N8a.1 (`rust_type(Type::Tuple)` fix) and N8a.2 (`TupleGet`/`TuplePut`
/// emit) are implemented.  When un-ignored, `50-tuples.loft` and `46-caveats.loft`
/// are removed from `SCRIPTS_NATIVE_SKIP`.
#[test]
fn native_tuple_script() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/50-tuples.loft");
    let job = prepare_native_test(entry)?;
    // Ok(false) = skipped (rustc absent, or Layer-2 low-space guard) — not a
    // failure; a real compile error returns Err.  Skip the run in that case.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    run_native_job(&job)
}

/// S35: native binary I/O script.
///
/// Runs `tests/scripts/20-binary.loft` through the native Rust backend end-to-end.
/// Exercises the Insert-return pattern (Set(rv, Insert([Set(_read_34, Null), Block])))
/// fixed in S35: output_set now hoists inner ops as statements before the assignment.
#[test]
fn native_binary_script() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/20-binary.loft");
    let job = prepare_native_test(entry)?;
    // Ok(false) = skipped (rustc absent, or Layer-2 low-space guard) — not a
    // failure; a real compile error returns Err.  Skip the run in that case.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    run_native_job(&job)
}

/// @PLN24 arc C — a `#c` binding calls a real C symbol from native-compiled loft,
/// with no Rust wrapper and no rustc in any library.
///
/// Bound to **libc**, deliberately: it is linked into every Rust binary, so this
/// proves the whole path — declaration, typed `extern "C"`, marshalling, call —
/// with no build step and nothing to install.
///
/// `atoi("-1")` is the cell that matters. A C `int` return read back as a bare
/// `u64` — what a signature-blind caller does — is 4294967295, a plausible large
/// positive that every `>= 0` check accepts; it is the shape that silently
/// defeated `loft-libs-net`'s `server::listen`. It comes back as -1 here only
/// because the declared width goes into the extern, so rustc truncates at the
/// ABI and the cast then sign-extends. That is the plan's invariant, executing.
///
/// `write(1, ptr, count)` covers the other half: a loft `vector` crosses as a
/// pointer AND a count, because C carries no length.
/// The POSIX `write(2)` binding as the generated sources below spell it.
const POSIX_WRITE: &str = r#"#c "write" "long(int, const void*, size_t)""#;

/// Spell a generated `#c` source for THIS host. A no-op off Windows.
///
/// Windows has `write(2)`'s behaviour but not its NAME: the CRT exports it as
/// `_write`, so a declaration naming `write` resolves to nothing there and the
/// call faults — measured, `` `#c` symbol 'write' not found ``.
///
/// Only the symbol is branched, because only the symbol differs. `long` is
/// genuinely right for the return on both: POSIX `write` gives `ssize_t`
/// (64-bit on LP64) and `_write` gives `int` (32-bit), which is exactly what C
/// `long` means on each — one of the places where naming the platform's own
/// width is the correct binding rather than a portability bug. The count
/// narrows to `unsigned int` for the same reason: that is `_write`'s third
/// parameter, where POSIX takes `size_t`.
fn for_host(src: &str) -> String {
    if cfg!(windows) {
        src.replace(
            POSIX_WRITE,
            r#"#c "_write" "long(int, const void*, unsigned int)""#,
        )
    } else {
        src.to_string()
    }
}

#[test]
fn native_c_binding_calls_libc() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let path = std::env::temp_dir().join("loft_pln24_c_binding.loft");
    std::fs::write(
        &path,
        for_host(
            "pub fn c_strlen(s: text) -> integer;   #c \"strlen\" \"size_t(const char*)\"\n\
         pub fn c_atoi(s: text) -> integer;     #c \"atoi\" \"int(const char*)\"\n\
         pub fn c_abs(v: integer) -> integer;   #c \"abs\" \"int(int)\"\n\
         pub fn c_write(fd: integer, v: vector<u8>) -> integer;  #c \"write\" \"long(int, const void*, size_t)\"\n\
         fn main() {\n\
         \x20 println(\"len {c_strlen(\"hello\")}\");\n\
         \x20 println(\"neg {c_atoi(\"-1\")}\");\n\
         \x20 println(\"abs {c_abs(-7)}\");\n\
         \x20 b: vector<u8> = [];\n\
         \x20 for ch in \"hi\\n\" { b += [(ch as integer) as u8? ?? (0 as u8)] }\n\
         \x20 println(\"wrote {c_write(1, b)}\");\n\
         }\n",
        ),
    )?;
    let job = prepare_native_test(&path)?;
    // Ok(false) = skipped (rustc absent / low space) — not a failure.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    let out = std::process::Command::new(&job.binary).output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("len 5"), "strlen: {stdout}");
    assert!(
        stdout.contains("neg -1"),
        "a 32-bit C return must sign-extend, not come back as 4294967295 — the \
         declared width is what makes that work: {stdout}"
    );
    assert!(stdout.contains("abs 7"), "abs: {stdout}");
    // `\r\n` -> `\n` before comparing. The bytes really do differ: the C runtime
    // opens fd 1 in TEXT mode on Windows, so the `write(1, "hi\n", 3)` this test
    // makes arrives as `hi\r\n`. Measured with `{stdout:?}` after the CI log —
    // which normalises line endings — showed output that looked identical to a
    // passing run. The translation is the platform behaving as documented, not
    // the binding losing a byte, and the claim under test is that the vector
    // crossed as pointer + count, which `wrote 3` is what settles.
    let stdout = stdout.replace("\r\n", "\n");
    assert!(
        stdout.contains("hi\n") && stdout.contains("wrote 3"),
        "a vector must cross as pointer + count: {stdout:?}"
    );
    Ok(())
}

/// @PLN24 arc B — the interpreter calls the same C symbols, and answers the
/// SAME thing.
///
/// The interpreter has no compiler at the call site, so it resolves the symbol
/// and calls it through the fixed per-arity trampolines the architecture probe
/// validated. This test is the parity half: identical stdout from both
/// backends, cell for cell, which is the bar a `#c` binding has to meet before
/// it can be said to work at all. Before arc B, `strlen("hello")` compiled
/// under `--interpret` and answered **7562**.
///
/// The `neg` cell carries the whole reason the declaration has a signature: the
/// interpreter reads a raw `u64` return register, so `atoi("-1")` arrives as
/// 4294967295 unless the DECLARED width truncates and re-extends it.
#[test]
fn interpreted_and_native_c_bindings_agree() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let path = std::env::temp_dir().join("loft_pln24_c_parity.loft");
    std::fs::write(
        &path,
        for_host(
            "pub fn c_strlen(s: text) -> integer;   #c \"strlen\" \"size_t(const char*)\"\n\
         pub fn c_atoi(s: text) -> integer;     #c \"atoi\" \"int(const char*)\"\n\
         pub fn c_abs(v: integer) -> integer;   #c \"abs\" \"int(int)\"\n\
         pub fn c_write(fd: integer, v: vector<u8>) -> integer;  #c \"write\" \"long(int, const void*, size_t)\"\n\
         fn main() {\n\
         \x20 println(\"len {c_strlen(\"hello\")}\");\n\
         \x20 println(\"neg {c_atoi(\"-1\")}\");\n\
         \x20 println(\"abs {c_abs(-7)}\");\n\
         \x20 b: vector<u8> = [];\n\
         \x20 for ch in \"hi\\n\" { b += [(ch as integer) as u8? ?? (0 as u8)] }\n\
         \x20 println(\"wrote {c_write(1, b)}\");\n\
         }\n",
        ),
    )?;
    let interp = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&path)
        .output()?;
    let out = String::from_utf8_lossy(&interp.stdout).into_owned();
    let err = String::from_utf8_lossy(&interp.stderr);
    assert!(interp.status.success(), "interpret failed: {out}\n{err}");
    assert!(out.contains("len 5"), "{out}");
    assert!(
        out.contains("neg -1"),
        "the interpreter reads a RAW return register — without the declared \
         width this is 4294967295: {out}"
    );
    assert!(out.contains("abs 7") && out.contains("wrote 3"), "{out}");

    // The parity half. `prepare_native_test` may skip (no rustc / low space),
    // and a skipped comparison must not read as agreement.
    let job = prepare_native_test(&path)?;
    if !compile_native_job(&job, &find_loft_rlib())? {
        return Ok(());
    }
    let native = std::process::Command::new(&job.binary).output()?;
    let nout = String::from_utf8_lossy(&native.stdout);
    assert_eq!(
        out, nout,
        "the two backends must answer identically, cell for cell"
    );
    Ok(())
}

/// @PLN24 arc D — the composition matrix, against a REAL C library, on both
/// backends.
///
/// Arcs A-C proved the mechanism against libc, which is already in the process.
/// This is the shape a library actually has: a package that declares
/// `[c] libs`, a `.so` built by nothing but `cc`, and a binding per cell of the
/// plan's mapping table — every integer width, a text argument, a vector as
/// pointer + count, the opaque-handle open/read/bump/close cycle, and the
/// 7-argument call that straddles the SysV register/stack boundary.
///
/// The assertion is that the two backends produce **identical** output, because
/// that is the only claim worth making: each one alone can be plausibly wrong
/// in a way the other is not.
///
/// Skips when `cc` is absent — the fixture is C, and a machine without a C
/// compiler cannot build it. It does NOT skip silently on a failed build: that
/// is a real failure.
#[test]
fn c_binding_matrix_against_a_declared_library() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = std::process::Command::new("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln24_matrix.loft");
    std::fs::write(
        &prog,
        "use lcabi;\n\
         fn main() {\n\
         \x20 println(\"i64 {lc_i64(lc_i64(1234567890123))}\");\n\
         \x20 println(\"neg {lc_neg_i32(1)}\");\n\
         \x20 println(\"len {lc_strlen(\"loft\")}\");\n\
         \x20 println(\"byte {lc_byte_at(\"loft\", 0)}\");\n\
         \x20 v: vector<integer> = [10, 20, 30];\n\
         \x20 println(\"vec {lc_i64_sum(v)}\");\n\
         \x20 h = lc_open(1000);\n\
         \x20 println(\"handle {lc_read(h)} {lc_bump(h, 7)}\");\n\
         \x20 lc_close(h);\n\
         \x20 println(\"arity7 {lc_arity7(1,1,1,1,1,1,1)}\");\n\
         \x20 t = lc_static_text();\n\
         \x20 println(\"text {t} {t.len()}\");\n\
         \x20 println(\"textarg {lc_strlen(lc_static_text())}\");\n\
         \x20 println(\"cat [{lc_static_text()}][{lc_static_text()}]\");\n\
         \x20 println(\"some {lc_maybe_text(1)}\");\n\
         \x20 println(\"none {lc_opt_text(0) ?? \"<null>\"}\");\n\
         \x20 println(\"latin1 {lc_latin1_text().len()}\");\n\
         \x20 c_text_positions();\n\
         }\n\
         // A text return has to reach EVERY value position, because the caller\n\
         // needs a destination record handed to it and only some positions were\n\
         // routed at first: a struct literal, a vector literal, a field write, a\n\
         // `match` subject and a comparison all lower to `w += producer()`, which\n\
         // is a different emission site from a plain local assignment. Missing it\n\
         // is not a slow path for a `#c` binding — there is no non-destination\n\
         // sibling to fall back to, so it SIGSEGV'd the interpreter while\n\
         // `--native` stayed correct. One function per position, so a regression\n\
         // names the position it broke.\n\
         struct CRec { name: text, n: integer }\n\
         fn c_text_positions() {\n\
         \x20 o = CRec{name: lc_static_text(), n: 1};\n\
         \x20 println(\"p-lit {o.name}\");\n\
         \x20 o.name = lc_maybe_text(1);\n\
         \x20 println(\"p-field {o.name}\");\n\
         \x20 if lc_static_text() == \"loft/c-abi\" { println(\"p-cmp ok\") } else { println(\"p-cmp NO\") }\n\
         \x20 m = match lc_static_text() { \"loft/c-abi\" => lc_maybe_text(1), _ => \"-\" };\n\
         \x20 println(\"p-match {m}\");\n\
         \x20 v: vector<text> = [lc_static_text(), lc_maybe_text(1)];\n\
         \x20 println(\"p-vlit {v[0]} {v[1]}\");\n\
         \x20 println(\"p-cond {if lc_static_text().len() > 3 { lc_static_text() } else { \"-\" }}\");\n\
         \x20 for i in 0..2 { println(\"p-loop{i} {lc_static_text()}\") }\n\
         \x20 s = lc_static_text() + \"!\";\n\
         \x20 println(\"p-concat {s}\");\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    // Hand-computed, not copied from a run: `lc_i64` is self-inverse, the
    // vector sum is position-weighted (10*1 + 20*2 + 30*3), and arity7's
    // arguments carry distinct prime weights (2+3+5+7+11+13+17).
    for want in [
        "i64 1234567890123",
        "neg -1",
        "len 4",
        "byte 108",
        "vec 140",
        "handle 1000 1007",
        "arity7 58",
        // @PLN24 arc D — the `char *` return. Hand-computed: "loft/c-abi" is 10
        // characters, so C's byte count and loft's character count agree on it
        // (`textarg` re-crosses the answer to prove the copy is NUL-terminated,
        // not merely non-empty). NULL reads as loft null. "caf\xE9" is 3 ASCII
        // bytes plus one invalid UTF-8 byte, which becomes ONE replacement
        // character — 4, not 3 (dropped) and not 5 (bytes counted as characters).
        "text loft/c-abi 10",
        "textarg 10",
        "cat [loft/c-abi][loft/c-abi]",
        "some here",
        "none <null>",
        "latin1 4",
        // Every value position, each named so a failure says which one broke.
        "p-lit loft/c-abi",
        "p-field here",
        "p-cmp ok",
        "p-match here",
        "p-vlit loft/c-abi here",
        "p-cond loft/c-abi",
        "p-loop0 loft/c-abi",
        "p-loop1 loft/c-abi",
        "p-concat loft/c-abi!",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "the two backends must agree cell for cell against a real library"
    );
    Ok(())
}

/// @PLN128 arc C / C106 — the `#c` arity ceiling is ONE contract, and both
/// backends are held to it.
///
/// This is a regression guard for an asymmetry that was silent for a long time:
/// the check ran behind `if !native_mode`, so `--native` bound and correctly
/// CALLED a 14-slot C function while the interpreter refused the same
/// declaration. A library author could ship that and only a downstream consumer
/// would find out — `loft debug` is the interpreter, so the bindings you could
/// not debug were exactly the ones with no other way in.
///
/// Both halves of the boundary are pinned, because either one alone can pass
/// while the contract is broken: at `MAX_C_ARITY` both backends must CALL and
/// agree on the value, and one past it both must REFUSE. The expected sums are
/// position-weighted (argument `i` counts `i`), so a trampoline that dropped or
/// reordered an argument gives a different number rather than a plausible one.
#[test]
fn the_c_arity_ceiling_is_the_same_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let max = loft::c_signature::MAX_C_ARITY;
    let dir = std::env::temp_dir().join("loft_pln128_arity");
    let src = dir.join("pkg/arity/src");
    std::fs::create_dir_all(&src)?;

    // One C function at the ceiling and one past it, each weighting argument i
    // by i+1 so a dropped or reordered argument gives a different number rather
    // than a plausible one.
    let mut csrc = String::new();
    for n in [max, max + 1] {
        let params = (0..n)
            .map(|i| format!("long long a{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let body = (0..n)
            .map(|i| format!("a{i}*{}", i + 1))
            .collect::<Vec<_>>()
            .join(" + ");
        csrc.push_str(&format!("long long ar{n}({params}) {{ return {body}; }}\n"));
    }
    std::fs::write(dir.join("arity.c"), &csrc)?;
    // The manifest keeps the LINUX spelling on every host: `platform::lib_variants`
    // translates `libarity.so` to `arity.dll` for Windows and `libarity.dylib` for
    // macOS, and both backends resolve it through that one home.  Only what gets
    // BUILT is host-specific.
    let libname = if cfg!(target_os = "macos") {
        "libarity.dylib"
    } else {
        "libarity.so"
    };
    if cfg!(windows) {
        // TWO artifacts, because on Windows the two backends need different files
        // and Unix gets away with one.  `--interpret` LoadLibrary's the fixture at
        // run time, which only a DLL can satisfy; `--native` links it, and a DLL is
        // not linkable on its own — MSVC wants the import library beside it, named
        // exactly `<stem>.lib` because `add_c_library_flags` passes `-l arity`.
        // That is `platform::shim_implib_args`' rule, called here rather than
        // respelled, so the fixture cannot drift from what loft actually asks for.
        let dll = dir.join("arity.dll");
        let implib = dir.join("arity.lib");
        let cc = std::process::Command::new("cc")
            .args(["-O1", "-shared", "-o"])
            .arg(&dll)
            .arg(dir.join("arity.c"))
            .args(loft::platform::shim_implib_args(
                &implib.to_string_lossy(),
                loft::platform::host_lib_os(),
            ))
            .output()?;
        assert!(
            cc.status.success(),
            "the arity fixture must build: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
        assert!(
            implib.exists(),
            "`cc -shared` must also write the import library {} — without it the \
             --native link cannot resolve `-l arity`",
            implib.display()
        );
    } else {
        let cc = std::process::Command::new("cc")
            .args(["-O1", "-fPIC", "-shared", "-o"])
            .arg(dir.join(libname))
            .arg(dir.join("arity.c"))
            .output()?;
        assert!(
            cc.status.success(),
            "the arity fixture must build: {}",
            String::from_utf8_lossy(&cc.stderr)
        );
    }
    std::fs::write(
        dir.join("pkg/arity/loft.toml"),
        format!(
            "[library]\nname = \"arity\"\nversion = \"0.1.0\"\n\n[c]\nlibs = \"../../{libname}\"\n"
        ),
    )?;
    let sig = |n: usize| {
        let lp = (0..n)
            .map(|i| format!("p{i}: integer"))
            .collect::<Vec<_>>()
            .join(", ");
        let cs = vec!["int64_t"; n].join(", ");
        format!("pub fn ar{n}({lp}) -> integer;\n#c \"ar{n}\" \"int64_t({cs})\"\n")
    };
    // Sum of i^2 for i in 1..=n — hand-computable, and different for n and n+1.
    let want = |n: usize| (1..=n).map(|i| i * i).sum::<usize>();
    let run = |backend: &str, prog: &std::path::Path| -> std::io::Result<(String, String)> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(dir.join("pkg"))
            .arg(prog)
            .output()?;
        Ok((
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    };

    // Pass 1 — CALL SITES. The binding lives in a dependency; the program calls
    // it. At the ceiling both backends must call it and agree on the value; one
    // past it, both must refuse.
    for (n, expect_ok) in [(max, true), (max + 1, false)] {
        std::fs::write(src.join("arity.loft"), sig(n))?;
        let call = (1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let prog = dir.join(format!("call{n}.loft"));
        std::fs::write(
            &prog,
            format!("use arity;\nfn main() {{ println(\"R {{ar{n}({call})}}\") }}\n"),
        )?;
        let mut refused = Vec::new();
        for backend in ["--interpret", "--native"] {
            let (stdout, stderr) = run(backend, &prog)?;
            if expect_ok {
                assert!(
                    stdout.contains(&format!("R {}", want(n))),
                    "{backend} at the ceiling ({n} slots) must call and answer {}: \
                     stdout={stdout:?} stderr={stderr:?}",
                    want(n)
                );
            } else {
                assert!(
                    stderr.contains("c-binding-not-interpretable"),
                    "{backend} past the ceiling ({n} slots) must refuse: \
                     stdout={stdout:?} stderr={stderr:?}"
                );
            }
            refused.push(stderr.contains("c-binding-not-interpretable"));
        }
        // The point of the guard: not that each backend behaves, but that they
        // behave the SAME. The old bug passed a per-backend check.
        assert_eq!(
            refused[0], refused[1],
            "the two backends disagreed about {n} C slots"
        );
    }

    // Pass 2 — DECLARATION in code the author OWNS, never called. This is what
    // puts the error in front of the person who can fix it; before arc C the
    // only check was at the call site, so the author never saw it at all.
    let owned = dir.join("owned.loft");
    std::fs::write(
        &owned,
        format!(
            "{}fn main() {{ println(\"declared, never called\") }}\n",
            sig(max + 1)
        ),
    )?;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&owned)
            .output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("c-binding-not-interpretable"),
            "{backend} must refuse an over-ceiling DECLARATION in owned code even \
             when nothing calls it: {stderr:?}"
        );
    }

    // ...but a DEPENDENCY declaring one the program never calls must still load:
    // a consumer cannot edit someone else's declaration, so it must not fail
    // their build. Mirrors how `superseded_fold_diagnostics` scopes itself.
    std::fs::write(
        src.join("arity.loft"),
        format!("{}{}", sig(max), sig(max + 1)),
    )?;
    let call = (1..=max)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let prog = dir.join("dep_ok.loft");
    std::fs::write(
        &prog,
        format!("use arity;\nfn main() {{ println(\"R {{ar{max}({call})}}\") }}\n"),
    )?;
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run(backend, &prog)?;
        assert!(
            stdout.contains(&format!("R {}", want(max))),
            "{backend}: a dependency's over-ceiling binding that is never called must \
             not break the build: stdout={stdout:?} stderr={stderr:?}"
        );
    }
    Ok(())
}

/// @PLN128 — the three shapes every numeric library (BLAS, LAPACK, FFTW, HDF5)
/// is actually made of, bound through `#c` and checked on both backends.
///
/// The load-bearing cell is `lc_daxpy`: **C writes its result THROUGH a
/// caller-supplied pointer**, which is how every BLAS and LAPACK routine
/// returns anything at all. If loft could not see those writes the numeric
/// stack would not be bindable, so this is the property the whole plan rests
/// on and it gets a guarantee probe rather than a one-off measurement.
///
/// The expected values are the ones `lc_selftest.c` computes in C (`make
/// check`), not numbers copied from a loft run — agreement between two loft
/// backends is not evidence that either matches C.
///
/// Skips when `cc` is absent, like its sibling above; a failed BUILD is a real
/// failure, not a skip.
#[test]
fn numeric_array_shapes_cross_identically_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = std::process::Command::new("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln128_numeric.loft");
    std::fs::write(
        &prog,
        "use lcabi;\n\
         fn main() {\n\
         \x20 a: vector<float> = [1.5, 2.25, 4.0];\n\
         \x20 println(\"dsum {lc_dsum_scaled(a)}\");\n\
         // C writes back through `y`. `y` is READ AFTERWARDS, which is also what\n\
         // keeps it alive across the call — a vector whose last use is the `#c`\n\
         // call itself is freed at that point and C's pointer dangles.\n\
         \x20 y: vector<float> = [1.0, 2.0, 3.0];\n\
         \x20 x: vector<float> = [10.0, 20.0, 30.0];\n\
         \x20 lc_daxpy(y, x, 1200);\n\
         \x20 println(\"daxpy {y[0]*1000.0} {y[1]*1000.0} {y[2]*1000.0}\");\n\
         // A Fortran scalar-by-reference against a C function that DOES take a\n\
         // count, so this one spends two slots. The Fortran cells below spend\n\
         // one, and that is the signature's decision rather than the type's.\n\
         \x20 s: vector<integer> = [6];\n\
         \x20 println(\"scalar {lc_scalar_ref(s)}\");\n\
         // The idiom the float refusal prescribes, on a COMPUTED double rather\n\
         // than a literal an author converted by hand.\n\
         \x20 v: vector<float> = [2.5];\n\
         \x20 out: vector<float> = [0.0];\n\
         \x20 lc_shim_scale(out, v);\n\
         \x20 println(\"shim {out[0]*1000.0}\");\n\
         \x20 fortran();\n\
         }\n\
         // @PLN128 arc D — the shape every real numeric library actually has:\n\
         // each argument a BARE pointer, no counts anywhere.  Before the count\n\
         // became the signature's decision, none of this was reachable — the\n\
         // honest declaration was refused for arity, and the shape loft insisted\n\
         // on delivered each count where the callee expected the next pointer,\n\
         // which SIGSEGV'd the interpreter and produced nothing under --native.\n\
         fn fortran() {\n\
         \x20 n: vector<integer> = [3];\n\
         \x20 al: vector<float> = [2.0];\n\
         \x20 be: vector<float> = [10.0];\n\
         \x20 x: vector<float> = [1.5, 2.25, 4.0];\n\
         \x20 y: vector<float> = [100.0, 200.0, 400.0];\n\
         \x20 lc_daxpby(n, al, x, be, y);\n\
         \x20 println(\"daxpby {y[0]*1000.0} {y[1]*1000.0} {y[2]*1000.0}\");\n\
         // `dgemm_` at full width: thirteen by-reference arguments, thirteen C\n\
         // slots.  This is the routine the ceiling was sized around.\n\
         \x20 d: vector<integer> = [2];\n\
         \x20 a2: vector<float> = [1.0, 2.0, 3.0, 4.0];\n\
         \x20 b2: vector<float> = [5.0, 6.0, 7.0, 8.0];\n\
         \x20 c2: vector<float> = [100.0, 200.0, 300.0, 400.0];\n\
         \x20 lc_dgemm(\"N\", \"N\", d, d, d, al, a2, d, b2, d, be, c2, d);\n\
         \x20 println(\"dgemm {c2[0]} {c2[1]} {c2[2]} {c2[3]}\");\n\
         // The two `char *` arguments have to land where the callee reads them.\n\
         // Asserting only the product above would pass with them misplaced,\n\
         // because the fixture computes the same product either way — it reports\n\
         // an unsupported transpose instead, so this cell is what pins them.\n\
         \x20 c3: vector<float> = [100.0, 200.0, 300.0, 400.0];\n\
         \x20 lc_dgemm(\"T\", \"N\", d, d, d, al, a2, d, b2, d, be, c3, d);\n\
         \x20 println(\"dgemm-t {c3[0]}\");\n\
         // Counted and bare in ONE signature, arranged so a left-to-right walk\n\
         // that grabs a count whenever an integer follows a pointer gives it to\n\
         // the wrong vector and answers a different number.\n\
         \x20 v2: vector<float> = [1.5];\n\
         \x20 w2: vector<float> = [2.0, 3.0];\n\
         \x20 println(\"split {lc_split(v2, 7, w2)}\");\n\
         \x20 elements();\n\
         }\n\
         // @PLN128 arc E — one cell per element WIDTH loft may hand over, each\n\
         // reader position-weighted in C so a stride that disagrees with loft's\n\
         // answers a different number rather than the right one by luck.  This\n\
         // is the half the declaration check lets through; the half it refuses\n\
         // is `a_vector_element_must_match_the_c_pointee`.\n\
         fn elements() {\n\
         \x20 a32: vector<u32> = [1000, 2000, 3000];\n\
         \x20 println(\"u32 {lc_u32_dot(a32)}\");\n\
         \x20 a16: vector<u16> = [10, 20, 30];\n\
         \x20 println(\"u16 {lc_u16_dot(a16)}\");\n\
         \x20 a8: vector<u8> = [1, 2, 200];\n\
         \x20 println(\"u8 {lc_u8_dot(a8)}\");\n\
         \x20 ac: vector<character> = ['A', 'B', 'C'];\n\
         \x20 println(\"char {lc_char_dot(ac)}\");\n\
         \x20 ab: vector<boolean> = [true, false, true];\n\
         \x20 println(\"bool {lc_bool_dot(ab)}\");\n\
         \x20 af: vector<single> = [0.5 as single, 1.25 as single, 2.5 as single];\n\
         \x20 println(\"single {lc_f32_dot(af)}\");\n\
         // The level-1 BLAS *function* shape: the answer comes back BY VALUE in\n\
         // an SSE register.  Refused until the caller grew a float-returning\n\
         // rung, which is what made `ddot_`/`dnrm2_`/`dasum_` need an ANSI-C\n\
         // shim each.  Both widths, because a C `float` return is a single in\n\
         // that register and reading those bits as a double is a denormal.\n\
         \x20 n3: vector<integer> = [3];\n\
         \x20 dx: vector<float> = [1.5, 2.5, 4.0];\n\
         \x20 dy: vector<float> = [4.0, 8.0, 16.0];\n\
         \x20 println(\"ddot {lc_ddot(n3, dx, dy)}\");\n\
         \x20 sx: vector<single> = [1.5 as single, 2.5 as single, 4.0 as single];\n\
         \x20 sy: vector<single> = [4.0 as single, 8.0 as single, 16.0 as single];\n\
         \x20 println(\"sdot {lc_sdot(n3, sx, sy)}\");\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 1.5 + 2.25 + 4.0 = 7.75, scaled by 1000.
        "dsum 7750",
        // a = 1.2; y[i] += a * x[i] over [1,2,3] and [10,20,30].
        "daxpy 13000 26000 39000",
        "scalar 42",
        "shim 5000",
        // Every value below is computed in C by `lc_selftest.c`, not read off a
        // loft run: agreement between two loft backends is not evidence that
        // either matches C.  y := 2*x + 10*y over [1.5, 2.25, 4] and
        // [100, 200, 400].
        "daxpby 1003000 2004500 4008000",
        // Column-major 2x2: 2*(A*B) + 10*C with A = [[1,3],[2,4]],
        // B = [[5,7],[6,8]], C = [[100,300],[200,400]].
        "dgemm 1046 2068 3062 4092",
        "dgemm-t -1",
        // 1.5*100 + 7*10 + (2*1 + 3*2), scaled by 1000.
        "split 228000",
        // @PLN128 arc E — one per element width, position-weighted so a wrong
        // stride cannot answer the right number.  All six are in
        // `lc_selftest.c` too, so C agrees with itself before loft is asked.
        "u32 14000",
        "u16 140",
        "u8 605",
        "char 398",
        "bool 4",
        "single 10500",
        // The float RETURN, both widths.  1.5*4 + 2.5*8 + 4*16 = 90.
        "ddot 90",
        "sdot 90",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "the two backends must agree on every numeric shape"
    );
    Ok(())
}

/// @PLN24 arc D — a C `char *` comes back as loft `text`, identically on both
/// backends, from libc alone (no fixture, no library to install).
///
/// `strerror` is the shape a real binding meets: borrowed storage the caller
/// must not free, a plain `int` argument, and an answer the C library owns. The
/// interpreter copies out of a raw return register and `--native` copies out of
/// a typed `*const c_void`; the two arrive at the same text or this crossing is
/// two mappings rather than one.
#[test]
fn a_c_string_return_crosses_identically_on_both_backends() -> std::io::Result<()> {
    let path = std::env::temp_dir().join("loft_pln24_c_textret.loft");
    std::fs::write(
        &path,
        "pub fn c_strerror(n: integer) -> text;   #c \"strerror\" \"char*(int)\"\n\
         pub fn c_strlen(s: text) -> integer;     #c \"strlen\" \"size_t(const char*)\"\n\
         fn main() {\n\
         \x20 e = c_strerror(2);\n\
         \x20 println(\"len {c_strlen(e)} chars {e.len()}\");\n\
         \x20 println(\"arg {c_strlen(c_strerror(2))}\");\n\
         }\n",
    )?;
    let mut seen = Vec::new();
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&path)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // Not an exact string: `strerror(2)` is locale-dependent (it is "No such
        // file or directory" in the C locale). What IS pinned is that the text
        // survived the crossing — a non-empty answer whose byte length C agrees
        // with, which a truncated or NUL-terminated-at-zero copy fails.
        assert!(
            !stdout.contains("len 0 ") && !stdout.contains("arg 0"),
            "{backend}: an empty `strerror` means the crossing dropped the text:\n{stdout}"
        );
        seen.push(stdout);
    }
    assert_eq!(
        seen[0], seen[1],
        "the two backends must bring back the same text"
    );
    Ok(())
}

/// @PLN24 arc G — `c_library_available` is symbol-granular, not file-granular.
///
/// The cell that matters: a library that LOADS but does not export a symbol the
/// package declared. A file-granular answer says yes here and the call then
/// faults — the version-skew hole that makes a naive query worse than none, and
/// the reason the answer checks symbols at all.
///
/// The library is built here rather than mocked, and the control is the call
/// that WORKS (`sk_present(41)` is 42, by hand): without it a `false` could just
/// mean nothing loaded, which is the vacuous pass this test exists to refuse.
// @speed 1.1
#[test]
fn an_available_library_must_export_what_was_declared() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the fixture library is built by `cc` here
    }
    let dir = std::env::temp_dir().join(format!("loft_skew_{}", std::process::id()));
    let pkg = dir.join("pkg/skewlib/src");
    std::fs::create_dir_all(&pkg)?;
    let src = dir.join("old.c");
    std::fs::write(&src, "long sk_present(long v) { return v + 1; }\n")?;
    let so = dir.join("libskew.so");
    let built = std::process::Command::new("cc")
        .args(["-O2", "-fPIC", "-shared", "-o"])
        .arg(&so)
        .arg(&src)
        .output()?;
    assert!(built.status.success(), "fixture library must build");

    // Forward slashes, because this path is about to be pasted into two string
    // literals — a TOML basic string and a loft one — and a Windows path is full
    // of escape sequences to both. `C:\Users\…` made the generated library fail
    // to lex at all: `error: Unknown escape sequence` at the `\U`. Escaping for
    // each syntax separately would work; using a separator neither treats as
    // special is simpler, and Windows accepts `/` everywhere loft passes this on
    // (`lib_variants` already splits a directory off on either separator).
    let so_str = so.to_string_lossy().replace('\\', "/");
    std::fs::write(
        dir.join("pkg/skewlib/loft.toml"),
        format!(
            "[library]\nname = \"skewlib\"\nversion = \"0.1.0\"\n\n[c]\noptional-libs = \"{so_str}\"\n"
        ),
    )?;
    std::fs::write(
        pkg.join("skewlib.loft"),
        format!(
            "pub fn sk_present(v: integer) -> integer;  #c \"sk_present\" \"long(long)\"\n\
             // Declared, and absent from this vintage of the library.\n\
             pub fn sk_newer(v: integer) -> integer;    #c \"sk_newer\" \"long(long)\"\n\
             pub const SKEW_SONAME = \"{so_str}\";\n\
             pub fn skew_ok() -> boolean {{ return c_library_available(SKEW_SONAME); }}\n"
        ),
    )?;
    let script = dir.join("probe.loft");
    std::fs::write(
        &script,
        "use skewlib;\nfn go() {\n  println(\"ok={skew_ok()}\");\n  println(\"call={sk_present(41)}\");\n}\ngo();\n",
    )?;

    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(dir.join("pkg"))
            .arg(&script)
            .output()?;
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {s}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            s.contains("call=42"),
            "{backend}: the control must prove the library IS loaded and callable:\n{s}"
        );
        assert!(
            s.contains("ok=false"),
            "{backend}: a loadable library missing a declared symbol is NOT available:\n{s}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// @PLN23 — one uniform interface over four different C libraries.
///
/// `dump <D: SqlDb>` and `seed <D: SqlDb>` in the fixture never name a backend,
/// and every backend runs them unchanged. That is the whole claim, and it is not
/// a small one: sqlite STEPS a prepared statement, libpq MATERIALISES the result
/// and indexes it by (row, col), libmariadb streams rows as `char **`, and
/// duckdb materialises and is read by (col, row) with a caller-frees string.
/// Four result models behind one cursor.
///
/// **sqlite is the cell that keeps this honest.** It needs no server, so a
/// machine with no database still proves the interface, the bindings, the shim
/// loft compiled, and SQL NULL staying distinct from the empty string. Only
/// postgres and mariadb are conditional, and a skip is printed and recognised —
/// never silently counted as a pass, which is how a dead binding would otherwise
/// look green everywhere.
///
/// **duckdb is the fourth, and it is here because of @PLN24 arc G.** It was
/// proven once before and left out of the tree: no distro packages it and its
/// `.so` is 70 MB, so a REQUIRED declaration would have made every machine
/// running this test fetch it. Declared `[c] optional-libs` it costs nothing —
/// the fixture builds and runs without it, and says so — which is what makes
/// keeping a fourth backend in the tree cheap rather than a vendoring decision.
// @speed 2.1
#[test]
fn one_sql_interface_drives_four_different_c_libraries() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    // Which backends actually ran their assertions. A skip and a pass are the
    // same colour, so the count is checked at the end: this test used to gate
    // each backend on `libsqlite3.so.0` existing under `/lib/x86_64-linux-gnu`
    // | `/usr/lib` | `/usr/lib64`, and BOTH the spelling and the directories are
    // Linux's. On macOS `/usr/lib` exists but holds `libsqlite3.dylib`, so every
    // conditional cell — including sqlite, written to be the unconditional one —
    // skipped in silence and the test passed on macOS for months having opened
    // no database. The availability question now has ONE home, loft's own
    // `c_library_available`, which translates the declared soname to the host's
    // spelling; a mode that cannot run says `SKIP` and is counted here.
    let mut ran: Vec<&str> = Vec::new();
    // No availability question is asked from out here. Every backend answers it
    // with `c_library_available` — true only when the library opens AND every
    // declared symbol resolves — and prints `SKIP`. A second question asked from
    // the harness could only be the weaker, file-granular one, and two answers
    // to one question is how an unusable library got called (loft#770).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("uniform.loft");
    let run = |backend: &str, mode: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .env("LOFT_SQLDB_MODE", mode)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // Exit status in the message, for the same reason as the sites above: a
        // Windows binary that cannot find a DLL dies before `main` with both
        // streams empty, and without the code the message says nothing at all.
        assert!(
            out.status.success(),
            "{backend}/{mode} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };

    // The three rows every backend is given, rendered by the SAME generic code:
    // a value, SQL NULL, and the empty string.
    let expect = "[ada] <null> [] ";

    // @PLN23 S4 — the prepared-statement line, and every token in it is a
    // separate claim:
    //
    //   p=2      two holes, and the DRIVER's own parameter count agreed with the
    //            parser's (each backend fails `prepare` outright when they differ)
    //   [ada] <null> []   value / SQL NULL / empty string, still three answers
    //                     — now arriving through the BIND path rather than as
    //                     literals in the statement text
    //   ['); DROP TABLE loft_p; --]
    //            the cell that matters. Spliced into SQL this closes the VALUES
    //            list and drops the table; bound, it is stored verbatim. That the
    //            SELECT after it returns rows at all is the proof the table
    //            survived, so this assertion cannot pass vacuously.
    //   hit=4    the same hostile text found its own row by EQUALITY, which it
    //            could only do by crossing intact as data rather than as syntax.
    //   big=1000 a value far past the 256-byte column buffer mariadb's result
    //            binds start with, round-tripped at full length — the truncation
    //            re-fetch is exercised, not merely written.
    //
    // @PLN23 H6 — the identifier, the ONE hole that is not a value. Four tokens,
    // two of which move in opposite directions if the type stops carrying it:
    //
    //   ident=4  a `SqlIdent` named the TABLE the query reads, so it reached the
    //            statement as SYNTAX. No placeholder stands for a table name, so
    //            a `SqlIdent` that went down the bind path instead would not have
    //            found the wrong row — it would have failed to prepare at all.
    //            The quoting is per-dialect and this is where that is measured:
    //            giving mariadb the ANSI quote the other three use answers
    //            `FAIL q8 … error in your SQL syntax … near '"loft_p" WHERE`,
    //            because it reads a double-quoted name as a string literal. The
    //            backtick is not a preference, and choosing the quote at ASSEMBLY
    //            time rather than when the hole was filled is what allows it.
    //   refused=true   the same construction given `loft_p; DROP TABLE loft_p; --`
    //            is refused AT CONSTRUCTION — before any statement was built
    //            around it, and long before a server could be asked about it.
    //   ran=false      the statement built on the refused identifier has no text.
    //            `statement` answers `text?` and every backend must discharge it,
    //            so nothing is sent.
    //   alive=true     and the table is still there. Vacuous only if `dump`
    //            itself were broken, which the `[ada] <null> []` cells rule out.
    let bound = "p=2 [ada] <null> [] ['); DROP TABLE loft_p; --] hit=4 big=1000 \
                 ident=4 refused=true ran=false alive=true";

    // @PLN23 T1–T3 — the transaction line, from one generic `transact<D: SqlDb>` that
    // names no backend. It lands before the object mapping because writing a collection
    // non-atomically is not a smaller step, it is a wrong one.
    //
    //   nested=false      a `db_begin` INSIDE a transaction is REFUSED. The silent
    //                     no-op is the dangerous version: the inner "rollback" would
    //                     discard the OUTER transaction's work, and nothing afterwards
    //                     can tell.
    //   rows=0/1          rollback DISCARDED the row, commit KEPT it. The pair is what
    //                     makes the cell non-vacuous — three of these four fields move
    //                     when the transaction is a fiction.
    //   stray=false/false commit or rollback with nothing open is refused.
    //
    // Proven to FIRE by replacing sqlite's three methods with `return true` (a backend
    // that reports transactions it never opens): `nested=true rows=1/2 stray=true/true`.
    let tx = "begin=true/true nested=false rollback=true commit=true rows=0/1 stray=false/false";

    // @PLN23 H7 — procedures, from one generic `procedures<D: SqlDb>` that names no
    // backend. Two of the four put the definition in the SERVER's catalogue
    // (`CREATE OR REPLACE PROCEDURE`, then `CALL`) and two keep it in the PROCESS
    // and prepare it on call; this line is what says a caller cannot tell which.
    //
    //   deployed=true  the definition was accepted. The body's parameters are typed
    //                  by the holes' own loft types, so a procedure is declared by
    //                  writing its statement rather than declaring a signature twice.
    //   called=true    running it did the work, and `rows=1` is that work seen from
    //                  OUTSIDE the procedure — the pair is what makes the cell
    //                  non-vacuous. Proven to fire by replacing sqlite's `db_call`
    //                  with `return true`: `guard=true rows=0`, two fields moving.
    //   guard=false    the same procedure called with the wrong NUMBER of values is
    //                  refused before anything is sent.
    //   ctl=false      a two-statement body is refused AT DEPLOY, everywhere. sqlite
    //                  and duckdb have no procedural language; mariadb and postgres
    //                  each have one and they are not the same one (SQL/PSM vs
    //                  plpgsql), so there is no such body a uniform API could carry.
    //                  Proven to fire by making `procedural` never refuse: `ctl=true`.
    let proc = "deployed=true called=true guard=false rows=1 ctl=false";

    // sqlite — no server, so whenever the library is here a failure is real.
    //
    // The availability question is the PROGRAM's to answer, exactly as it is for
    // postgres and maria below. Asking it from out here needs a second, WEAKER
    // question — "does a file with this name load" — and that one says yes for
    // an unrelated library sharing the translated name, which is how a hostile
    // `sqlite3.dll` on PATH turned a skip into an access violation on Windows
    // (loft#770). `c_library_available`, which the backend uses, is true only
    // when every declared symbol resolves. One home for the fact; the harness
    // reads the verdict off stdout.
    let s = run("--interpret", "sqlite")?;
    if s.contains("SKIP") {
        assert!(
            s.contains("not installed"),
            "an absent library must be REPORTED, not inferred from silence:\n{s}"
        );
    } else {
        ran.push("sqlite");
        assert!(
            s.contains(&format!("sqlite {expect}")),
            "sqlite must render value / NULL / empty distinctly:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite bound {bound}")),
            "sqlite: a bound value must reach the server as DATA, never as syntax:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite tx {tx}")),
            "sqlite: rollback must discard, commit must persist, and a nested begin \
             must be REFUSED:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite proc {proc}")),
            "sqlite: a procedure kept in the PROCESS must deploy, call and do the \
             work, and refuse a body needing a procedural language:\n{s}"
        );
        // @PLN133 P3 — a bound float must come back BIT-IDENTICAL.
        //
        // **sqlite is pinned at 6 of 7, which is its real answer.**  It sends a
        // float as TEXT (no `#c` path carries a `double` by value) and its own
        // text→REAL converter rounds `-5.196972490273514e-183` one ULP wrong,
        // where `sqlite3_bind_double` on the same value is right — measured
        // directly.  The fix needs `#c` float support (@PLN128 E3): it cannot go
        // in the shim, which is deliberately free of sqlite symbols so it links
        // where the optional library is absent (@PLN24 arc G).  The other three
        // parse correctly and are held to 7/7 below.
        //
        // This cell read 7/7 for a while, and that was an ARTEFACT rather than a
        // pass: `floats` is itself generic, so under loft#791 its write loop and
        // its read loop saw the same corrupted vector and agreed with each other.
        // Fixing #791 made the guard honest and the real defect reappeared.  Keep
        // the value that exposes it; when E3 lands, raise this to 7/7.
        //
        // The per-backend READ expression is still load-bearing for a different
        // reason: sqlite renders a `REAL` as `%!.15g`, so reading the column
        // naively loses the low bits of values that ARE stored correctly.
        assert!(
            s.contains("sqlite float wrote=7 exact=6/7 inlined=false plain=true"),
            "sqlite: a bound float must round-trip exactly (see @PLN133 P3):\n{s}"
        );
        assert_eq!(
            run("--native", "sqlite")?,
            s,
            "both backends, one interface"
        );
    }

    // postgres and mariadb — conditional, and a skip is recognised as a skip.
    // The condition is the library's own answer plus a reachable server; both
    // arrive as `SKIP` on stdout rather than being guessed at from out here.
    for mode in ["postgres", "maria"] {
        let out = run("--interpret", mode)?;
        if out.contains("SKIP") {
            continue; // library absent, or no server reachable here
        }
        ran.push(mode);
        assert!(
            out.contains(&format!("{mode} {expect}")),
            "{mode} must render the same three cells as sqlite:\n{out}"
        );
        // Byte-identical to sqlite's, from the same generic `bound<D: SqlDb>`:
        // three placeholder dialects, three bind APIs, three result models, one
        // answer. A backend that quietly concatenated would differ HERE and
        // nowhere else, which is what makes the line worth comparing whole.
        assert!(
            out.contains(&format!("{mode} bound {bound}")),
            "{mode}: the bound statement must give the same answer as sqlite:\n{out}"
        );
        // The same generic `transact<D: SqlDb>`, byte-identical to sqlite's answer:
        // three servers with three transaction implementations, one contract.
        assert!(
            out.contains(&format!("{mode} tx {tx}")),
            "{mode}: transactions must behave exactly as sqlite's do:\n{out}"
        );
        // The interesting half of H7: this backend put the definition in its own
        // catalogue and CALLed it, sqlite kept it in the process — and the line is
        // byte-identical. Where a procedure lives is not something a caller can see.
        assert!(
            out.contains(&format!("{mode} proc {proc}")),
            "{mode}: a server-side procedure must give the same answer as the \
             process-side emulation:\n{out}"
        );
        // @PLN133 P3 — a bound float must come back BIT-IDENTICAL.
        //
        // Every backend sends a float as TEXT (no `#c` path carries a `double`
        // by value), so each is relying on its server's text→double conversion
        // being correctly rounded.  Three of the four are.  **sqlite is not**:
        // it loses the last bit of `-5.196972490273514e-183`, measured directly
        // against `sqlite3_bind_double`, which gets it right.  Fixing it needs
        // `#c` float support (@PLN128 E3) — it cannot go in the shim, which is
        // deliberately free of sqlite symbols so it links where the optional
        // library is absent (@PLN24 arc G).
        //
        // Both of these servers parse correctly, so they are held to 7/7 —
        // sqlite is the odd one out and is pinned separately, above.
        assert!(
            out.contains(&format!(
                "{mode} float wrote=7 exact=7/7 inlined=false plain=true"
            )),
            "{mode}: a bound float must round-trip exactly (see @PLN133 P3):\n{out}"
        );
        assert_eq!(
            run("--native", mode)?,
            out,
            "{mode}: both backends must agree"
        );
    }

    // @PLN24 arc G — duckdb, declared `[c] optional-libs`.
    //
    // This cell is unconditional ON PURPOSE, and it is the one that proves the
    // arc: every mode above needs its C library installed to run at all, and
    // before arc G a declared-but-absent library failed the `--native` LINK, so
    // a program that never called into it did not build. Here the program is
    // expected to build and run either way — the only question is which of the
    // two answers it gives.
    let out = run("--interpret", "duckdb")?;
    let native = run("--native", "duckdb")?;
    assert_eq!(
        native, out,
        "duckdb: both backends must agree about an optional library"
    );
    if out.contains("SKIP") {
        // libduckdb absent — the interesting half. The program still ran.
        assert!(
            out.contains("not installed"),
            "an absent optional library must be REPORTED, not inferred from silence:\n{out}"
        );
    } else {
        // libduckdb present — then it is held to exactly the bar the other three meet,
        // and the arc-G property costs it no leniency. Proven on 1.5.5 with the library
        // reachable via `LD_LIBRARY_PATH`; no system install is needed, and none is
        // assumed here, which is why this stays conditional.
        ran.push("duckdb");
        assert!(
            out.contains(&format!("duckdb {expect}")),
            "duckdb must render the same three cells as sqlite:\n{out}"
        );
        assert!(
            out.contains(&format!("duckdb bound {bound}")),
            "duckdb: a bound value must reach the server as DATA, never as syntax:\n{out}"
        );
        assert!(
            out.contains(&format!("duckdb tx {tx}")),
            "duckdb: rollback must discard, commit must persist, and a nested begin \
             must be REFUSED — `BEGIN TRANSACTION` is its own spelling:\n{out}"
        );
        assert!(
            out.contains(&format!("duckdb proc {proc}")),
            "duckdb: the process-side procedure registry is SHARED with sqlite's, so \
             its answer must be sqlite's:\n{out}"
        );
    }

    // The guard that would have caught the Linux-shaped probe.
    //
    // Everything above is conditional, so with no cell running this test is a
    // green that asserted nothing — which is exactly what it was on macOS.
    // Naming the backends that ran turns a silent evaporation into a readable
    // one, and requiring sqlite on Linux pins the reference platform: it is the
    // cell with no server to be unreachable, so there it can only be missing if
    // something upstream broke.
    println!("@PLN23 backends exercised: {ran:?}");
    if cfg!(target_os = "linux") {
        assert!(
            ran.contains(&"sqlite"),
            "sqlite must run on Linux — it needs no server, so a skip here means \
             the library went missing or the availability question broke, and a \
             pass with it skipped asserts nothing. Exercised: {ran:?}"
        );
    }
    Ok(())
}

/// @PLN138 — two cursors from one connection, and the resource each one owns.
///
/// The claim this whole plan exists to make. @PLN23 kept the cursor as state ON
/// the connection because loft interfaces had no associated types, so a contract
/// could not say "and this connection yields a cursor of ITS OWN kind"; the price
/// was that a connection held exactly one, and a second `db_select` silently
/// replaced the first. `type Rows: SqlRows` states it, and the fixture measures
/// it — every function in it is generic over `SqlDb` and names no backend.
///
/// **UNCONDITIONAL, like the pure-derivation cell above and for the same reason.**
/// sqlite needs no server, so a machine with no database still proves the
/// guarantee. Every other SQL test here skips where a library or a server is
/// missing; a guarantee whose gate can evaporate into a green is not one.
///
/// Two claims, and the second is the one a fixture alone cannot make:
///
///   - the VALUES, hand-computed per cell inside the script. Row 2's name is SQL
///     NULL and row 3's is the empty string, so the interleaved pairing shows
///     each cursor held its own position rather than reading the other's row.
///   - the RELEASE, answered by sqlite itself. Forty cursors are abandoned
///     mid-walk and the connection is then closed; `sqlite3_close` reports
///     `SQLITE_BUSY` while any statement on it is unfinalized, so a scope end
///     that failed to release is a return code rather than a leak inferred from
///     memory growth. Verified to FAIL with the hook removed.
///
/// The script counts its own failures and withholds the summary line when there
/// are any — a `check` that only printed would let a run with failures in it
/// still print `two cursors ok`, which is exactly the false green this file's
/// other tests are written against.
#[test]
fn one_connection_yields_two_independent_cursors() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("two_cursors.loft");
    let mut ran = false;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            continue;
        }
        ran = true;
        assert!(
            stdout.contains("two cursors ok") && !stdout.contains("FAIL"),
            "{backend}: a connection must yield independent cursors:\n{stdout}"
        );
        // A wrong free is refused rather than performed, so it costs a printed
        // line and not a wrong answer — which means the values above can all
        // agree while the ownership underneath is broken.
        assert!(
            !stdout.contains("BUG (#"),
            "{backend}: cursors must not provoke an internal fault:\n{stdout}"
        );
    }
    assert!(
        ran,
        "sqlite needs no server, so a skip here means the library went missing \
         or the availability question broke — and a pass with it skipped asserts \
         nothing about the guarantee"
    );
    Ok(())
}

/// @PLN133 S2–S5 — one table definition, derived and reconciled, with no
/// database anywhere.
///
/// Drop the sqldb fixtures' `note:` advisories before comparing two backends' stdout.
///
/// Each of those fixtures deletes its temp database on the way out and prints
/// `note: … was not removable` when the file survives.  That reports on the ENVIRONMENT,
/// not on the answer the oracle is about.  On Windows a database file's handle can outlive
/// the process that held it for a moment, so the first backend's run reports the note and
/// the second — run straight after, against the same path — does not; the two stdouts then
/// differ by a line neither backend computed, and a green leg went red at a commit that had
/// changed nothing about either path (loft#834).
///
/// Only the advisory is dropped.  Every line carrying a value, a count, or a driver hit is
/// still compared, and each backend is still checked against `expect` with the note in
/// place — so a fixture that genuinely stopped cleaning up on both backends is still
/// visible, and the failure this filters is the one that is not a difference in behaviour.
fn answer_without_housekeeping(stdout: &str) -> String {
    stdout
        .lines()
        .filter(|l| !l.trim_start().starts_with("note: "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The whole point of `TableDef` being a VALUE is that it is built two ways —
/// derived from a loft type, or read back from a database — and consumed four,
/// with nothing downstream allowed to ask which. That makes the derivation
/// testable with no connection at all: the hand-built `have` definitions in the
/// script stand in for what `introspect` will read, and they are the same shape,
/// which is the invariant rather than a shortcut.
///
/// **So this cell is UNCONDITIONAL**, and that is what it is for. Every other
/// SQL test in this file skips where a library or a server is missing; this one
/// runs on any machine, so the derivation the reader and the writer must agree
/// on has a gate that cannot evaporate into a green.
///
/// The DDL it asserts is HAND-WRITTEN per dialect. Comparing one generator
/// against another proves they agree, which is not the question.
#[test]
fn one_table_definition_derives_reconciles_and_renders() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        // The `schema` package holds no `#c` at all, but it sits beside the
        // backends in one lib directory and `sql` is compiled with it.
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("schema_pure.loft");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("schema ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // A leak here is not cosmetic. This derivation runs on the LAZY FETCH
        // path, once per miss, so a record left behind per call is a traversal
        // that grows the heap for as long as it runs.
        assert!(
            !stdout.contains("not freed"),
            "{backend}: the derivation must leave nothing behind:\n{stdout}"
        );
    }
    Ok(())
}

/// @PLN133 S6 — the definition loft WROTE, read back out of sqlite's catalogue.
///
/// The pure gate above proves the derivation against hand-built definitions.
/// This proves the hand-built ones were the right shape: `derive` from a loft
/// type and `introspect` from the database produce one value, and `reconcile`
/// matches them. That round trip is what requirement 2 rests on — a writer and a
/// reader deriving their SQL separately agree only until they do not, and
/// nothing else checks it.
///
/// **Run twice, and the second run is the one that matters.** Into an EMPTY
/// database, where loft creates the schema; and against a table made by hand
/// with a scrambled column order, a float kept in a `VARCHAR`, a boolean kept in
/// `TEXT` and one extra column, where loft follows it. The first run passes even
/// if `reconcile` always agrees.
#[test]
fn a_table_loft_wrote_and_a_table_loft_found_are_one_value() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("schema_live.loft");

    // Every token is a separate claim, and three of them move in different
    // directions if the round trip is a fiction:
    //
    //   created cols=4 ix=1 …   loft's own DDL came back as four columns AND the
    //           index its collection kind implies. @PLN129 refuses a bind whose
    //           lookup no index serves, so a writer that omitted it would build
    //           a database its own reader cannot open.
    //   declared flag=INTEGER   sqlite has no boolean type, so loft wrote one as
    //           INTEGER — and the BINDING is what turns it back into a boolean.
    //           That is why `flag conversion=ConvBoolean` is beside it: the pair
    //           is the claim, and either alone would look right while wrong.
    //   followed cols=5 bound=true   a table loft did not write, with the columns
    //           in a different ORDER and an extra one, is readable. Column order
    //           in a foreign table means nothing and must never be read as
    //           meaning.
    //   varchar score conversion=ConvFloat   a float kept in a VARCHAR is the
    //           same conversion as one kept in a REAL, because every driver hands
    //           the value over as text.
    //   noindex bound=false     …and the refusal NAMES the column, because a
    //           refusal a DBA cannot act on is only a slower failure.
    //   extra bound=true write=false   ONE table, two verdicts. An unknown NOT
    //           NULL column with no default is perfectly readable and impossible
    //           to INSERT into.
    //
    // @PLN23 S7c adds a migration that RUNS, and the rows are the gate:
    //
    //   grown rows 1|ada|null;2|grace|null;   the two original rows, untouched,
    //           and the added column reading NULL rather than a fabricated
    //           value. A plan changes a table's SHAPE and never its CONTENT, and
    //           this line is where that stops being a sentence.
    //   grown before bound=false … / grown after bound=true write=true   the
    //           binding that refused now works, and the plan came off the SAME
    //           comparison that refused — migration and binding disagree about
    //           nothing.
    //   moved rows 7|seven;8|eight;   a DECLARED rename, and the values
    //           travelled with the name. Undeclared it would be an ADD plus an
    //           orphan, which is what "indistinguishable from a drop plus an
    //           add" means.
    let expect = [
        "created cols=4 ix=1 bound=true write=true why=",
        "declared flag=INTEGER score=REAL",
        "flag conversion=ConvBoolean",
        "followed cols=5 bound=true write=true why=",
        "varchar score conversion=ConvFloat",
        "noindex bound=false why=table noix_person has no index on id",
        "extra bound=true write=false why=column tenant is NOT NULL with no default",
        // @PLN23 S7c — the migration ran, and the CONTENT is the claim. A plan
        // that changed content would show up as a value that moved.
        "grown before bound=false why=table grown has no column memo",
        "grown plan ready=true steps=1",
        "grown after bound=true write=true why=",
        "grown rows 1|ada|null;2|grace|null;",
        "moved plan ready=true steps=1",
        "moved rows 7|seven;8|eight;",
        "schema_live ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            // The availability question is the PROGRAM's, answered by
            // `c_library_available` — true only when every declared symbol
            // resolves. A skip is printed, never inferred from silence.
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        // Both backends, one derivation. A backend that derived its own would
        // differ HERE and nowhere else, which is what makes the whole output
        // worth comparing rather than each line.
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one table definition"
            ),
        }
    }
    Ok(())
}

/// @PLN23 S6 — the child tables a collection field implies, against a real
/// engine.
///
/// The pure gate proves the DERIVATION; this proves it was a schema an engine
/// accepts and a round trip that closes. That distinction is the plan's own
/// history: the cleanest version of the addressing rule — *the declared key
/// addresses an element* — was falsified by an `INSERT`, not by re-reading the
/// design (OBJECT_MAPPING.md § What the probe falsified).
///
/// Three lines, and each moves in a different direction if the address rule is
/// wrong:
///
///   docs 7|seven;9|nine;11|eleven   doc 11's tag vector is EMPTY and doc 9's
///           score vector is. A parent with no children is still a row, and a
///           write path that emitted a parent only when it had children would
///           lose it silently.
///   scores 7|0|10;7|1|20;11|0|30   the S6a shape, unchanged by S6b landing
///           beside it. Two collections of different kinds under one owner, and
///           the field path in the table NAME is what keeps them apart.
///   tags 7|0|a|1|~;7|1|b|2|;9|0|a|1|x;9|1|a|1|x;9|2|c|3|~   the S6b shape, and
///           three claims at once. Doc 9 holds the SAME tag twice: under a
///           key-addressed rule those collapse to one row, and under the ordinal
///           rule they are rows 0 and 1 — the falsified claim, standing as a
///           test. `a/1` is also doc 7's, so an owner column that went missing
///           MERGES two documents rather than losing anything. And `~` is SQL
///           NULL beside `b`'s empty string: not the same value, which is most
///           of why a binding exists.
///
/// @PLN23 S6c adds the two KEYED shapes, and the kind is the only thing that
/// decides between them:
///
///   seen ord=false ix=1 rank ord=true ix=2   a `hash` addresses by its declared
///           key, so it carries no ordinal and one index; a `sorted` takes the
///           ordinal and owes its key an index of its own. @PLN129 refuses a
///           bind whose lookup no index serves, so the second one is not
///           decoration — and a `sorted` sub-collection had neither it nor a
///           refusal before S6c.
///   seen 7|m1|10;7|m2|20;9|m1|99   `m1` under two owners. A hash key is unique
///           WITHIN its collection, and the owner column is the only thing
///           keeping the two apart.
///   rank 7|0|1|a;7|1|2|b;7|2|3|c;…   the steps went in as 3,1,2. The ordinal is
///           the COLLECTION's order, not the order a program added things in,
///           and only an out-of-order insertion can tell those apart.
///   beats 1|0|1|y;1|1|1|x;1|2|2|z   TWO elements of bar 1, surviving as rows 0
///           and 1. This is the `INSERT` that falsified the clean addressing
///           rule, kept as a test — and it needs `Beat` to have a second keyed
///           view, because a `sorted` whose element type has only one view
///           REPLACES on an equal key (measured, both backends).
///   byname 1|1|x;1|1|y;1|2|z   the SAME three records, addressed by the other
///           view's key. Only `beats` was written to; two keyed collections over
///           one element type are views of one record set (loft#843), so both
///           child tables hold all three.
///
/// @PLN23 S7 recurses: a collection inside a record ELEMENT is a table of its
/// own, addressed by (the root's key, the parent element's address, its own).
///
///   pieces 1|0|0|10|1|2;1|0|1|11|3|4;…;2|1|0|14|9|0
///           Two ledgers, two marks each, two pieces each — the smallest shape
///           where each of the THREE address levels is separately load-bearing.
///           Both ledgers have a mark `A` and both `A`s hold pieces 10 and 11,
///           so dropping `ledger_id` merges the ledgers, dropping `ord` merges
///           `A`'s pieces with `B`'s, and dropping `ord_2` collapses the pieces
///           within one mark. Three different wrong answers, and none of them is
///           "fewer rows than expected".
///   marks 1|0|A;1|1|B;2|0|A;2|1|C   the repeated label, one level up.
///   …|10|1|2   the last two columns are `at_row` / `at_col`: an INLINE struct
///           inside the grandchild's element, flattened (@PLN23 S7b). It has no
///           identity, so it has no table — and the write reads it through the
///           column's PATH, which is what `field_value(x, [outer, inner])` was
///           added to loft for.
///   notes 1|0|100;1|1|200;2|0|300   @PLN23 S7b-ii — a collection inside an
///           INLINE struct. `(ledger_id, ord)` and no more: `meta` has no
///           identity, so it adds to the table NAME and to nothing else, while
///           its sibling scalar `rev` flattens into the owner's own row
///           (`ledger 1|one|5`). A record ELEMENT in the same position DOES add
///           an ordinal — that is the distinction, and it is the mapping's one
///           rule (identity earns an address) applied one level in.
///   permute at_col=4,at_row=3,pid=11,ord_2=1,ord=0,ledger_id=1,   ONE row built from a REVERSED
///           definition. The address columns come out outer→inner, so the two
///           ordinals are always in depth order and a writer counting them in
///           ROW order agrees on every other line here. Reversing the columns is
///           the only thing that tells `ords[c.depth]` from a counter — without
///           it, `ColumnDef.depth` is a field no test could distinguish from one.
///
/// sqlite, so a machine with no database still runs it.
#[test]
fn a_collection_field_becomes_child_rows_a_real_engine_gives_back() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("children_live.loft");

    let expect = [
        "tables=5 parent=doc scores=doc_scores tags=doc_tags",
        "seen ord=false ix=1 rank ord=true ix=2",
        "docs   7|seven;9|nine;11|eleven",
        "scores 7|0|10;7|1|20;11|0|30",
        "tags   7|0|a|1|~;7|1|b|2|;9|0|a|1|x;9|1|a|1|x;9|2|c|3|~",
        "seen   7|m1|10;7|m2|20;9|m1|99",
        "rank   7|0|1|a;7|1|2|b;7|2|3|c;9|0|6|g;9|1|8|h",
        "beats  1|0|1|y;1|1|1|x;1|2|2|z",
        "byname 1|1|x;1|1|y;1|2|z",
        "depth  tables=4 grandchild=ledger_marks_pieces inline=ledger_meta_notes",
        "ledger 1|one|5;2|two|6",
        "marks  1|0|A;1|1|B;2|0|A;2|1|C",
        "pieces 1|0|0|10|1|2;1|0|1|11|3|4;1|1|0|12|5|6;1|1|1|13|7|8;2|0|0|10|1|2;2|0|1|11|3|4;2|1|0|14|9|0",
        "notes  1|0|100;1|1|200;2|0|300",
        "permute at_col=4,at_row=3,pid=11,ord_2=1,ord=0,ledger_id=1,",
        "children_live ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one set of child tables"
            ),
        }
    }
    Ok(())
}

/// @PLN133 S7 — one connection string, and the registry that turns it into a
/// connection every consumer can use.
///
/// `SqlDb` is satisfied by four unrelated types and loft interfaces are STATIC
/// dispatch, so no function can return "one of them". The registry is a
/// struct-enum that satisfies the interface itself — and the strongest claim in
/// the fixture is one nothing asserts: `uniform` there is generic over `SqlDb`
/// and is handed an `AnyDb`. If the enum did not satisfy the interface the
/// program would not compile, which is the entire question S7 had to answer.
///
/// **Unconditional**, like the pure schema gate beside it: this half opens no
/// library, so it cannot skip into a green that asserted nothing. The cells that
/// need a database live in the live gate below.
#[test]
fn one_connection_string_reaches_its_driver_and_a_refusal_behaves_like_one() -> std::io::Result<()>
{
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        // The registry holds no `#c` itself, but it names all four backends in
        // one type, so their shims are compiled with it.
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("registry_pure.loft");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("registry ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !stdout.contains("not freed"),
            "{backend}: a refused connection must leave nothing behind:\n{stdout}"
        );
    }
    Ok(())
}

/// @PLN133 S7 — the schema round trip, over a connection ONE STRING opened.
///
/// The pure gate above proves a string reaches its driver. This proves the
/// connection it produced is a connection: `introspect` — generic over `SqlDb`
/// and written before the registry existed — takes the enum unchanged, a cursor
/// walks rows through it, and a transaction lands on the same connection the
/// insert did.
///
/// **The cursor cell is the one that could quietly fail.** A variant holds a
/// COPY of the backend struct, so the handle `db_select` writes and `db_next`
/// reads has to live INSIDE the enum. A copy that came apart reads as an empty
/// result set — which looks exactly like an empty table.
#[test]
fn a_connection_the_registry_opened_is_a_connection() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("registry_live.loft");

    // Each token is a separate claim, and they fail in different directions:
    //
    //   connected backend=sqlite quote="   the STRING chose the driver, and the
    //           dialect came with it rather than being asked for separately.
    //   round trip cols=4 ix=1 bound=true   `derive` → `render` → `introspect` →
    //           `reconcile`, all over the enum. @PLN129 refuses a bind whose
    //           lookup no index serves, so `ix=1` is what makes the table usable.
    //   cursor names=ada grace alan   bound INSERTs and a walked cursor, so the
    //           mutation reached inside the variant. An enum holding a stale copy
    //           answers "" here, which is what an empty table also answers.
    //   float naive=false dialect=true   sqlite renders a REAL as %!.15g, so the
    //           portable `SELECT score` loses the low bits of a full-mantissa
    //           double. The PAIR is the claim: `dialect=true` alone could mean the
    //           naive read was fine too, `naive=false` alone that the write failed.
    //   tx … rows=3/4   rollback discarded, commit kept. A db_begin that quietly
    //           did nothing answers 4/4 and one that discarded everything 3/3.
    let expect = [
        "connected backend=sqlite quote=\"",
        "round trip cols=4 ix=1 bound=true",
        "cursor names=ada grace alan",
        "float naive=false dialect=true",
        "tx begin=true/true rollback=true commit=true rows=3/4",
        "registry_live ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one registry"
            ),
        }
    }
    Ok(())
}

/// @PLN133 S9 — the same lazy read, down core's Rust source and down a LOFT
/// driver, over one database.
///
/// Core drives sqlite in Rust (`sql_source.rs` + `sql_query.rs`, 913 lines) and
/// the loft library drives four backends behind one `SqlDb` interface. S10
/// deletes the Rust; this is the measurement that would let it. The claim is not
/// "the loft driver works" — it is that **the two paths are indistinguishable to
/// the program above them**.
///
/// Two element types of the same shape over two identical tables, in ONE program
/// bound to ONE connection string: `S9Rust` has no driver so core serves it,
/// `S9Loft` has one and S9's precedence rule sends it to loft. Every assertion is
/// made twice, once per path.
///
/// **The counts are the oracle, not the values.** A lazy read that fetched the
/// whole table would return exactly these values; only the trip count separates
/// it from an eager load, which is why @PLN129 asserts it. Here it is visible
/// from outside because the driver prints: three lookups reach the source and the
/// repeat of a resident key reaches none.
///
/// Nothing in the driver names a column. The table, the columns and the `WHERE`
/// all come from `derive(type_of(coll))` — the same `TableDef` a writer would
/// `render` into `CREATE TABLE` — which is requirement 2's one derivation doing
/// both jobs.
#[test]
fn a_lazy_read_gives_one_answer_down_rust_and_down_loft() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("s9_two_paths.loft");

    // Each token is a claim about BOTH paths at once, so a difference between
    // them shows up as an asymmetric line rather than as an absent one:
    //
    //   value rust=grace loft=grace   the derived SELECT found the same row.
    //   float rust=0.25 loft=0.25     sqlite renders a REAL as %!.15g, so a
    //           SELECT that did not wrap the column comes back almost right.
    //           `select_by_key` applies the dialect's read expression; core's
    //           Rust source does its own equivalent, and they must agree.
    //   identity rust=true loft=true  one record however it is reached — which is
    //           what makes the collection, not an identity map, the authority.
    //   resident 1/1 then touched 2/2  `len` counts what was TOUCHED, so an
    //           eager load would read 3 here on either path.
    //   absent true/true + clean [] [] a genuine absence is not a failure, and
    //           reporting it as one is the mirror of the bug arc C exists for.
    let expect = [
        "value rust=grace loft=grace",
        "float rust=0.25 loft=0.25",
        "identity rust=true loft=true",
        "resident rust=1 loft=1",
        "second rust=alan loft=alan",
        "touched rust=2 loft=2",
        "absent rust=true loft=true",
        "clean rust=[] loft=[]",
        "s9 ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed"),
                "an absent library must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        // THE oracle. Three lookups reach the loft driver — 42, 7 and the absent
        // 999 — and the repeat of 42 does not, because it hit the working set.
        // Every value above would be identical under an eager load; only this
        // would not.
        assert_eq!(
            stdout.matches("loft-driver key=").count(),
            3,
            "{backend}: 3 lookups reach the driver and a resident hit reaches \
             none:\n{stdout}"
        );
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one answer per path"
            ),
        }
    }
    Ok(())
}

/// @PLN133 S14 — THE GATE. Write rows, bind lazily to the SAME connection
/// string, traverse, and get back what was written.
///
/// The plan's three requirements as one program: one string switches every
/// consumer; a structure written is immediately readable through lazy loading;
/// and a loft type has ONE table definition — created where the database has
/// nothing, FOLLOWED where it already holds a table.
///
/// **It runs TWICE, and the second run is the one that matters.** Run 1 goes
/// into an empty database, where loft writes the schema. Run 2 goes into a table
/// made by hand — different column ORDER, the float kept in a `VARCHAR`, and an
/// extra column loft knows nothing about — where loft must follow it. Run 1
/// passes even if `reconcile` is a stub that always agrees; only run 2 proves
/// requirement 3.
///
/// **The trip count is the oracle the values cannot be.** A reader that fetched
/// the whole table would return exactly these rows. Only the number of trips
/// separates a lazy read from an eager one, so the driver prints one line per
/// trip: 42 reaches the database, the repeat of 42 does not, 7 does, and the
/// absent 999 does — three per run, six in all.
///
/// CI reaches sqlite only. `LOFT_SQLDB_MODE` selects the other three, and the
/// local four-backend run is written into the plan where it was measured
/// (doc/claude/TESTING.md § Database backends).
#[test]
fn a_structure_written_is_immediately_readable_through_one_connection_string() -> std::io::Result<()>
{
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("round_trip.loft");

    // Each token is one claim, and the `created`/`followed` prefix says which of
    // the two runs made it — so a design that only works on a table loft wrote
    // itself shows up as HALF the lines rather than as none.
    //
    //   value=grace float=0.25 flag=false  the row came back through a SELECT
    //           built from the same TableDef the INSERT was built from.
    //   identity=true                      one record however it is reached: a
    //           write through the second handle is visible through the first.
    //   resident=1 then touched=2          `len` counts what was TOUCHED, so an
    //           eager load would read 3.
    //   absent=true clean=[]               a genuine absence is not a failure —
    //           the mirror of the bug @PLN129 arc C exists for.
    //   digest=true                        @PLN23 S5: the record that came back
    //           IS the record that went in, compared through the same walk that
    //           WROTE it.  It sees what the tokens beside it cannot — those name
    //           only grace's fields and alan's NAME, so a driver returning
    //           `flag=false` for every row keeps every other claim (grace's flag
    //           genuinely is false) and fails on alan's digest alone.  Measured:
    //           that break leaves `created digest=true` and turns
    //           `created second=alan touched=2 digest=true` false.
    let expect = [
        "created value=grace float=0.25 flag=false",
        "created digest=true",
        "created identity=true resident=1",
        "created second=alan touched=2 digest=true",
        "created absent=true clean=[]",
        "followed value=grace float=0.25 flag=false",
        "followed digest=true",
        "followed identity=true resident=1",
        "followed second=alan touched=2 digest=true",
        "followed absent=true clean=[]",
        "round_trip ok",
    ];

    let mut first: Option<String> = None;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if stdout.contains("SKIP") {
            assert!(
                stdout.contains("not installed") || stdout.contains("cannot"),
                "an unreachable backend must be REPORTED:\n{stdout}"
            );
            return Ok(());
        }
        for line in expect {
            assert!(
                stdout.contains(line),
                "{backend}: expected `{line}` in:\n{stdout}"
            );
        }
        assert_eq!(
            stdout.matches("  trip key=").count(),
            6,
            "{backend}: three trips per run and none for a resident hit:\n{stdout}"
        );
        match &first {
            None => first = Some(stdout),
            Some(f) => assert_eq!(
                answer_without_housekeeping(f),
                answer_without_housekeeping(&stdout),
                "both backends, one round trip"
            ),
        }
    }
    Ok(())
}

/// @PLN23 S3 — the cursor model: a real result set, walked through a shim loft
/// compiled itself, with SQL NULL kept distinct from the empty string.
///
/// The whole vertical slice in one test — loft → libmariadb + a loft-built
/// ANSI-C shim → a live server → rows back — with no rustc anywhere.
///
/// **The `shim` mode is what keeps this honest.** The cursor needs a server, and
/// a test that merely skips when one is absent cannot tell "no database" from "a
/// dead binding", so it would report a broken shim as a pass on every machine
/// without MariaDB. The `shim` mode needs no server and still exercises the
/// pieces that can silently rot: the shim compiled, `char *` → `text?`, and a
/// NULL pointer arriving as loft null. Only the SERVER half is conditional.
#[test]
fn a_sql_cursor_walks_real_rows_and_keeps_null_apart_from_empty() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    if !present
        || std::process::Command::new("cc")
            .arg("--version")
            .output()
            .is_err()
    {
        return Ok(());
    }
    let libdir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Beside the fixture it needs, NOT in tests/scripts/ — that directory is swept
    // and every script there must run standalone, while this one needs `--lib`.
    let script = libdir.join("mariadb").join("cursor.loft");
    let run = |backend: &str, mode: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .env("LOFT_PLN23_MODE", mode)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // Exit status in the message, for the same reason as the sites above: a
        // Windows binary that cannot find a DLL dies before `main` with both
        // streams empty, and without the code the message says nothing at all.
        assert!(
            out.status.success(),
            "{backend}/{mode} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };

    // Always: the shim and the null crossing, no server involved.
    let shim = run("--interpret", "shim")?;
    assert!(
        shim.contains("shim null=true") && shim.contains("shim info=true"),
        "the shim must build and a null row must cross as loft null:\n{shim}"
    );
    assert_eq!(
        run("--native", "shim")?,
        shim,
        "both backends, one loft-built shim"
    );

    // Conditional: the cursor itself.
    let cursor = run("--interpret", "cursor")?;
    if cursor.contains("SKIP unreachable") {
        return Ok(()); // no server here; the shim half above still ran
    }
    for want in [
        "cols 3 rows 3",
        "row 1 ada NULL", // SQL NULL
        "row 2 grace [hi]",
        "row 3 kay []", // the empty string — NOT the same thing
        "done",
    ] {
        assert!(cursor.contains(want), "cursor missing `{want}`:\n{cursor}");
    }
    assert_eq!(
        run("--native", "cursor")?,
        cursor,
        "both backends must read the same rows"
    );
    Ok(())
}

/// @PLN23 S2 — the opaque-handle lifecycle against a real client library.
///
/// `MYSQL *` crosses as a loft `integer` holding the pointer, which is the
/// convention @PLN24 chose because loft has no type separating a handle from a
/// number. Three things have to hold: a handle comes back, it survives being
/// passed BACK in, and a failure carries C's own message across.
///
/// Deliberately **not** gated on a running server. A test that skips when the
/// database is absent cannot tell "no server" from "the binding is broken", and
/// would report a dead binding as a pass on every machine without MariaDB. So
/// the cells here need no server at all: `mysql_init` and `mysql_close` are
/// client-side, and connecting to a port nothing listens on exercises the error
/// path — the binding is proved either way, and only the SPEED of the failure
/// depends on the environment.
#[test]
fn a_c_library_handle_survives_the_round_trip_and_carries_its_error() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    if !present {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("loft_pln23_s2");
    let pkg = dir.join("mariadb").join("src");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(
        dir.join("mariadb").join("loft.toml"),
        "[library]\nname = \"mariadb\"\nversion = \"0.0.1\"\n\n[c]\nlibs = \"libmariadb.so.3\"\n",
    )?;
    std::fs::write(
        pkg.join("mariadb.loft"),
        // `unix_socket` is `integer`, not `text`: it has to be able to be NULL,
        // and loft text is non-null with no way to spell a null pointer.
        "pub fn db_init(h: integer) -> integer;  #c \"mysql_init\" \"void*(void*)\"\n\
         pub fn db_close(h: integer);            #c \"mysql_close\" \"void(void*)\"\n\
         pub fn db_errno(h: integer) -> integer; #c \"mysql_errno\" \"int(void*)\"\n\
         pub fn db_error(h: integer) -> text;    #c \"mysql_error\" \"const char*(void*)\"\n\
         pub fn db_connect(h: integer, host: text, user: text, pass: text, db: text, port: integer, sock: integer, flags: integer) -> integer;\n\
         #c \"mysql_real_connect\" \"void*(void*, const char*, const char*, const char*, const char*, int, const char*, long)\"\n",
    )?;
    let prog = dir.join("s2.loft");
    std::fs::write(
        &prog,
        "use mariadb;\n\
         fn main() {\n\
         \x20 h = db_init(0);\n\
         \x20 println(\"handle {h != 0}\");\n\
         \x20 // Port 1 has no MariaDB anywhere; the point is the error crossing.\n\
         \x20 c = db_connect(h, \"127.0.0.1\", \"nobody\", \"nothing\", \"\", 1, 0, 0);\n\
         \x20 println(\"refused {c == 0} errno {db_errno(h) != 0} msg {db_error(h).len() > 0}\");\n\
         \x20 db_close(h);\n\
         \x20 println(\"closed\");\n\
         }\n",
    )?;
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&dir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        "handle true",                      // a pointer came back as an integer
        "refused true errno true msg true", // and C's own diagnosis came back with it
        "closed",                           // the handle was still usable at the end
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    assert_eq!(run("--native")?, interp, "both backends, one C library");
    Ok(())
}

/// @PLN24 arc F / @PLN23 S1 — a `#c` binding reaches a real SYSTEM library, on
/// both backends, with no rustc and no dev headers.
///
/// The fixture proved the mechanism; a system library proves the parts a fixture
/// cannot have. `libmariadb.so.3` is a VERSIONED soname, and that is the whole
/// point of this test: `-l dylib=mariadb` makes the linker look for
/// `libmariadb.so`, the `-dev` symlink, while the interpreter `dlopen`s
/// `libmariadb.so.3`, the runtime file — so one declaration resolved to two
/// different files and the program ran interpreted and failed to LINK natively on
/// a machine where the library is plainly installed.
///
/// Skips when libmariadb is absent, because a machine without it says nothing.
#[test]
fn a_c_binding_reaches_a_versioned_system_library_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Present only if the runtime package is installed; the `-dev` symlink is
    // deliberately NOT required, which is the property under test.
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    if !present {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("loft_pln23_s1");
    let pkg = dir.join("mariadb").join("src");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(
        dir.join("mariadb").join("loft.toml"),
        "[library]\nname = \"mariadb\"\nversion = \"0.0.1\"\n\n[c]\nlibs = \"libmariadb.so.3\"\n",
    )?;
    std::fs::write(
        pkg.join("mariadb.loft"),
        "pub fn client_info() -> text;       #c \"mysql_get_client_info\" \"const char*(void)\"\n\
         pub fn client_version() -> integer; #c \"mysql_get_client_version\" \"long(void)\"\n",
    )?;
    let prog = dir.join("s1.loft");
    std::fs::write(
        &prog,
        "use mariadb;\nfn main() { println(\"{client_info()} {client_version()}\") }\n",
    )?;

    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&dir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    // Not an exact version — that is the machine's. What is pinned: the text came
    // back non-empty and the integer is a real version, so both crossings worked.
    let v: i64 = interp
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    assert!(
        v > 10000,
        "the client version must cross as a real number: {interp:?}"
    );
    assert_eq!(
        run("--native")?,
        interp,
        "both backends, one system library"
    );
    Ok(())
}

/// @PLN24 arc E — a wasm target gets a NAMED refusal, and only where a `#c`
/// binding is actually called.
///
/// This is an EMISSION test on purpose: it needs no wasm toolchain, so the
/// guarantee is checked on every machine that runs the suite rather than only on
/// one that can cross-compile. The end-to-end halves live in `html_wasm.rs`.
///
/// Three separate failures used to come out of here, and a reader could not tell
/// any of them from a bug in their own program:
///
/// * a symbol the wasm sysroot happens to export (`strlen`) LINKED — with a
///   warning — and trapped at the call (`signature_mismatch: strlen`), because
///   wasm32 is a third data model (ILP32) and the extern carried the host's
///   widths;
/// * one it does not export gave a raw `rust-lld: undefined symbol`;
/// * a package declaring `[c] optional-libs` gave `E0433: cannot find c_call in
///   loft` once per symbol — for bindings the program never called.
///
/// So the assertions are in three parts: the reachable call is refused by name,
/// nothing in the file reaches for `c_call` or declares a C extern, and the
/// availability tables survive (they are what makes `c_library_available`
/// compile, and asking it is the idiom the refusal points an author towards).
#[test]
fn a_c_binding_is_refused_by_name_on_a_wasm_target() {
    let dir = std::env::temp_dir().join(format!("loft_pln24_arce_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("arce.loft");
    std::fs::write(
        &src_path,
        // `used` is called; `unused` is only declared.  Both are `#c`.
        "fn used(s: text) -> integer;    #c \"strlen\" \"size_t(const char*)\"\n\
         fn unused(v: integer) -> integer; #c \"abs\" \"int(int)\"\n\
         fn main() { println(\"{used(\\\"hello\\\")}\") }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "the declaration itself must stay legal on every target: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let main_nr = p.data.def_nr("n_main");
    let till = p.data.definitions();

    let emit = |wasm: bool| -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut out = Output::new(&p.data, &state.database);
        out.wasm_wasi = wasm;
        out.output_native_reachable(&mut buf, 0, till, &[main_nr])
            .expect("emit");
        String::from_utf8(buf).expect("utf8")
    };

    // The calibration half: on the HOST target the same program emits the real
    // binding.  Without this a refusal that fired everywhere would read as a pass.
    let host = emit(false);
    assert!(
        host.contains("#[link_name = \"strlen\"]") && !host.contains("@PLN24 arc E"),
        "the host target must still emit the typed extern and no refusal"
    );

    let wasm = emit(true);
    assert!(
        wasm.contains("`used` is bound to the C symbol 'strlen' with #c")
            && wasm.contains("wasm (wasip2)")
            && wasm.contains("--native-wasm")
            && wasm.contains("@PLN24 arc E"),
        "the reachable call must be refused by name: {wasm}"
    );
    assert!(
        !wasm.contains("loft::c_call"),
        "nothing may reach for `c_call` — it is not in a wasm build, and this is \
         what broke a program for merely DECLARING an optional-library binding"
    );
    assert!(
        !wasm.contains("#[link_name = \"strlen\"]") && !wasm.contains("#[link_name = \"abs\"]"),
        "no C extern may be declared: one the sysroot satisfies links and then traps"
    );
    assert!(
        wasm.contains("static __C_LIBS") && wasm.contains("static __C_LIB_SYMS"),
        "the availability tables stay on every target — `c_library_available` \
         reads them, and it used to fail to compile under --html for want of them"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// @PLN24 arc D — loft compiles the ANSI-C shim itself, with `cc` and no rustc.
///
/// The plan's trade is that loft-core stays a generic linking tool and every
/// signature the fixed trampolines cannot express is wrapped in a few lines of C
/// the library ships. That trade only holds if loft can BUILD those lines —
/// otherwise the escape hatch is a claim, and the author's alternative is the
/// rustc toolchain `#c` exists to avoid.
///
/// One cell per shape that needs a shim: a `double` argument (a different
/// register file, so it crosses as its bit pattern), an out-parameter (two
/// answers, one return slot), and a caller-frees `char *` (loft never frees one,
/// so the shim owns the release). The float cell is hand-computed rather than
/// copied from a run — 2.5 × 4.0 is exactly 10.0, whose bit pattern is pinned
/// below, so a shim that returned a plausible-but-wrong double fails.
#[test]
fn loft_builds_the_ansi_c_shim_a_package_ships() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    let libdir = root.join("pkg");
    // Start from no artifact, so the build itself is what is under test.
    let _ = std::fs::remove_dir_all(libdir.join("lcshim").join("native-auto"));

    let prog = std::env::temp_dir().join("loft_pln24_shim.loft");
    std::fs::write(
        &prog,
        "use lcshim;\n\
         fn main() {\n\
         \x20 println(\"scale {shim_scale(4612811918334230528, 4616189618054758400)}\");\n\
         \x20 println(\"mod {shim_mod(17, 5)}\");\n\
         \x20 println(\"owned {shim_owned(7)}\");\n\
         }\n",
    )?;
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        // The EXIT STATUS belongs in the message, because the interesting Windows
        // failure is a SILENT one: a binary that links but cannot find its DLL at
        // load time dies with `STATUS_DLL_NOT_FOUND` (0xC0000135) having written
        // nothing at all, so stdout and stderr are both empty and the assertion
        // said nothing without this.
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 2.5 * 4.0 = 10.0 → 0x4024000000000000.
        "scale 4621819117588971520",
        "mod 2",
        "owned shim-7",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    // The artifact is the proof loft did the compiling: nothing else put it there.
    let built: Vec<_> = std::fs::read_dir(libdir.join("lcshim").join("native-auto"))
        .map(|d| d.filter_map(Result::ok).map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        !built.is_empty(),
        "loft must have built the shim into native-auto/"
    );

    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "a shim-backed binding must answer the same on both backends"
    );
    Ok(())
}

/// @PLN24 arc D — the return spellings a `#c` declaration must refuse, because
/// nothing at runtime would.
///
/// The plan's central measurement is that a wrong binding produces a plausible
/// answer rather than a failure, so every one of these is caught at the
/// declaration or not at all. Both backends must refuse in the same words: a
/// shape rejected by one and accepted by the other is the divergence this plan
/// exists to keep out.
#[test]
fn a_text_return_must_say_it_is_a_c_string() -> std::io::Result<()> {
    // (declaration, the words the refusal has to carry)
    let cases = [
        (
            "pub fn f() -> text;   #c \"lc_x\" \"void*(void)\"",
            "spell the return `char*`",
        ),
        (
            "pub fn f() -> text;   #c \"lc_x\" \"int(void)\"",
            "the loft declaration returns `text`",
        ),
        (
            // A `vector` return cannot be reconstructed from a pointer: C
            // carries no length, which is why an ARGUMENT crosses as a PAIR and
            // a return cannot cross at all.
            "pub fn f() -> vector<integer>;   #c \"lc_x\" \"char*(void)\"",
            "returns `vector<integer>` but the C signature returns `char *`",
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln24_c_textret_refuse.loft");
        std::fs::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                !out.status.success(),
                "{backend} must refuse `{decl}`: {stderr}"
            );
            assert!(
                stderr.contains(want),
                "{backend}: refusing `{decl}` must say `{want}`: {stderr}"
            );
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must say it identically"
        );
    }
    Ok(())
}

/// @PLN128 arc E — a `vector<T>` and the C pointee are two spellings of ONE
/// layout, and a declaration where they disagree is refused.
///
/// Every case here was a running program with wrong numbers in it, measured
/// against real OpenBLAS on both backends with exit 0 and no diagnostic:
///
/// * `vector<integer>` (8-byte) against LAPACK's `int *` pivot array — `dgesv_`
///   answered `ipiv = 8589934593, 0` where the pivots are `1, 2`.
/// * `vector<single>` (4-byte) against `double *` — `daxpy_` wrote 24 bytes
///   into a 12-byte loft vector, which is a write past the end of a store
///   allocation from a declaration loft accepted.
/// * `vector<i8>` / `vector<i16>` — loft stores narrow SIGNED elements as
///   `val - min`, so C reads every value shifted by 128 / 32768.
/// * `vector<text>` — the elements are loft's own heap handles, so C
///   dereferences a store offset as an address: immediate SIGSEGV.
///
/// None of them needs a library to be installed: the declaration is what is
/// wrong, so the refusal lands before anything is called. The counterpart —
/// every element width that DOES cross, checked against values C computes — is
/// in `numeric_array_shapes_cross_identically_on_both_backends`.
#[test]
fn a_vector_element_must_match_the_c_pointee() -> std::io::Result<()> {
    // (declaration, the words the refusal has to carry)
    let cases = [
        (
            "pub fn f(v: vector<integer>);   #c \"lc_x\" \"void(int*)\"",
            "striding 4 bytes",
        ),
        (
            "pub fn f(v: vector<single>);   #c \"lc_x\" \"void(double*)\"",
            "striding 8 bytes",
        ),
        // Same WIDTH, different class. The message must not claim an overrun
        // here — there isn't one — only that every value arrives different.
        (
            "pub fn f(v: vector<float>);   #c \"lc_x\" \"void(const int64_t*)\"",
            "the same width read as integers",
        ),
        (
            "pub fn f(v: vector<i16>);   #c \"lc_x\" \"void(const short*)\"",
            "loft stores narrow SIGNED elements as `val - min`",
        ),
        (
            "pub fn f(v: vector<text>);   #c \"lc_x\" \"void(const char *const *)\"",
            "loft's own heap handles",
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln128_elem_refuse.loft");
        std::fs::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                !out.status.success(),
                "{backend} must refuse `{decl}`: {stderr}"
            );
            assert!(
                stderr.contains(want),
                "{backend}: refusing `{decl}` must say `{want}`: {stderr}"
            );
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must say it identically"
        );
    }
    // `void*` is the opaque escape hatch and stays open: it is how `write(2)`
    // takes a `vector<u8>`, and it is the author saying "these are bytes".
    let path = std::env::temp_dir().join("loft_pln128_elem_opaque.loft");
    std::fs::write(
        &path,
        "pub fn f(v: vector<float>);   #c \"lc_x\" \"void(const void*, int64_t)\"\n\
         fn main() { println(\"x\") }\n",
    )?;
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--errors=compact")
            .arg(&path)
            .output()?;
        assert!(
            out.status.success(),
            "{backend} must accept an opaque `void*` pointee: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// @PLN128 Q5 — a RETAINING C API is bindable, over a C-owned buffer.
///
/// The plan/execute split is not an FFTW quirk: zlib's `z_stream` keeps
/// `next_in`/`next_out`, `sqlite3_bind_text(SQLITE_STATIC)` keeps the caller's
/// bytes, and every "context object" API is this shape. C holds a buffer
/// pointer across two calls the caller makes.
///
/// Bound with a **loft** vector that is a use-after-free (E6b): loft frees the
/// vector at its last loft-visible use, which is the call that handed the
/// pointer over, and C reads whatever took its place. Measured, both backends,
/// no fault and no diagnostic — and deliberately NOT asserted here, because
/// pinning the current output would lock the bug in.
///
/// Bound with a **C-owned** buffer it is ordinary, which is what this pins.
/// Nothing loft owns is retained; the buffer's lifetime is the handle's; and
/// the two copies live only for their own call, which is what `#c` already
/// guarantees. It needs no shim — `memcpy` is libc — and it is what FFTW's own
/// documentation tells C callers to do anyway, because `fftw_malloc` is what
/// gives the SIMD alignment.
///
/// The expected value is `lc_selftest.c`'s, computed in C.
#[test]
fn a_retaining_c_api_binds_over_a_c_owned_buffer() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = std::process::Command::new("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln128_retain.loft");
    std::fs::write(
        &prog,
        "use lcabi;\n\
         fn main() {\n\
         \x20 n = 3;\n\
         \x20 bytes = n * 8;\n\
         \x20 buf = lc_buf_alloc(bytes);\n\
         \x20 src: vector<float> = [1.5, 2.25, 4.0];\n\
         \x20 lc_load(buf, src, bytes);\n\
         // The plan RETAINS the buffer here and reads it in `lc_run` below —\n\
         // a later call, which is the shape that fails with a loft vector.\n\
         \x20 p = lc_plan(buf, n);\n\
         \x20 println(\"retained {lc_run(p)}\");\n\
         // And back out, so the round trip is closed rather than assumed.\n\
         \x20 back: vector<float> = [0.0, 0.0, 0.0];\n\
         \x20 lc_store(back, buf, bytes);\n\
         \x20 println(\"roundtrip {back[0]} {back[1]} {back[2]}\");\n\
         \x20 lc_plan_free(p);\n\
         \x20 lc_buf_free(buf);\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 1.5*1 + 2.25*2 + 4.0*3 = 18.0, scaled by 1000 — `lc_selftest.c`'s value.
        "retained 18000",
        "roundtrip 1.5 2.25 4",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "a retaining API must answer the same on both backends"
    );
    Ok(())
}

/// @PLN128 arc E — a float RETURN binds; a float ARGUMENT still does not.
///
/// The plan settled the two together and that was one decision too few. An
/// argument would need a trampoline per SUBSET of positions that are float —
/// `2^arity`, genuinely impossible — while the return is a single axis and
/// costs one more expansion of the same arity list. Refusing it cost the
/// level-1 BLAS *functions* (`ddot_`, `dnrm2_`, `dasum_`) and every LAPACK
/// auxiliary that answers a number, and the cure it prescribed was an ANSI-C
/// shim per routine: a C toolchain in the build of every numeric package.
///
/// The pairing is exact rather than widening: a C `float` leaves a SINGLE in
/// the return register, so binding it to loft `float` would read those bits as
/// a double and get a denormal.
#[test]
fn a_float_return_binds_and_a_float_argument_still_does_not() -> std::io::Result<()> {
    let cases = [
        (
            "pub fn f(v: vector<float>) -> integer;   #c \"lc_x\" \"double(const double*, int64_t)\"",
            Some("must return `float`, not `integer`"),
        ),
        (
            "pub fn f(v: vector<float>) -> float;   #c \"lc_x\" \"float(const double*, int64_t)\"",
            Some("must return `single`, not `float`"),
        ),
        (
            "pub fn f(v: integer) -> float;   #c \"lc_x\" \"int64_t(int64_t)\"",
            Some("a float comes back only from a C `float` or `double`"),
        ),
        (
            "pub fn f(v: float);   #c \"lc_x\" \"void(double)\"",
            Some("travels in an SSE register the caller does not write"),
        ),
        // The two that now bind. Nothing is called, so no library is needed —
        // the declaration either type-checks or it does not.
        (
            "pub fn f(v: vector<float>) -> float;   #c \"lc_x\" \"double(const double*, int64_t)\"",
            None,
        ),
        (
            "pub fn f(v: vector<single>) -> single;   #c \"lc_x\" \"float(const float*, int64_t)\"",
            None,
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln128_float_return.loft");
        std::fs::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            match want {
                Some(w) => {
                    assert!(
                        !out.status.success(),
                        "{backend} must refuse `{decl}`: {stderr}"
                    );
                    assert!(
                        stderr.contains(w),
                        "{backend}: refusing `{decl}` must say `{w}`: {stderr}"
                    );
                }
                None => assert!(
                    out.status.success(),
                    "{backend} must accept `{decl}`: {stderr}"
                ),
            }
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must answer `{decl}` identically"
        );
    }
    Ok(())
}

/// @PLN28: unbounded recursion in native-compiled code must surface loft's typed
/// `call stack overflow` (exit non-zero) — the same cap the interpreter enforces —
/// NOT the opaque Rust `fatal runtime error: stack overflow, aborting`.  The
/// generated `main` runs on a large-stack thread so the `MAX_CALL_DEPTH` guard in
/// `cr_call_push` fires cleanly before the OS stack is exhausted.
#[test]
fn native_deep_recursion_reports_clean_stack_overflow() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    // Write to a temp file, NOT tests/scripts/ (which the success-runners sweep —
    // an infinitely-recursing script would break them).
    let path = std::env::temp_dir().join("loft_native_stack_overflow_guard.loft");
    std::fs::write(
        &path,
        "fn recur(n: integer) -> integer { m = recur(n + 1); return m + 1; }\n\
         fn main() { x = recur(0); print(\"{x}\"); }\n",
    )?;
    let job = prepare_native_test(&path)?;
    // Ok(false) = skipped (rustc absent / low space) — not a failure.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    let out = std::process::Command::new(&job.binary).output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "deep recursion must exit non-zero; stderr: {stderr}"
    );
    assert!(
        stderr.contains("call stack overflow"),
        "expected loft's typed stack-overflow diagnostic; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("fatal runtime error"),
        "must NOT be the raw Rust stack-overflow abort; stderr: {stderr}"
    );
    Ok(())
}

/// N8a.3: native tuple-returning functions.
///
/// The same 50-tuples.loft script will include a tuple-returning function once
/// N8a.3 is implemented.  This is a placeholder: un-ignored together with
/// native_tuple_script when the updated script passes.
#[test]
fn native_tuple_return_script() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/50-tuples.loft");
    let job = prepare_native_test(entry)?;
    // Ok(false) = skipped (rustc absent, or Layer-2 low-space guard) — not a
    // failure; a real compile error returns Err.  Skip the run in that case.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    run_native_job(&job)
}

// Initiative 03 Phase 2: the native Moros editor driver lives at
// `lib/graphics/examples/moros_editor.loft` and compiles end-to-end
// via `loft --native --path <repo>/ --lib <repo>/lib`.  Manual
// verification on a machine with a display server: the binary opens
// a 1024×768 window rendering the 7×7 hex starter map; WASD orbits
// the camera; Esc exits.  An automated compile-regression needs the
// `prepare_native_test` harness to pass `--path <repo>` for sibling-
// module resolution (`use render;` works only from inside the
// graphics package's own src/ or examples/ dirs) — deferred to
// Phase 5 polish; see doc/claude/plans/03-native-moros-editor/.

/// Library packages whose tests do NOT yet compile/run under `--native`.
/// These are pre-existing native-codegen / binding / runtime gaps (NOT linkage
/// gaps — the `#native`-crate linkage was fixed via
/// `native_utils::add_native_extern_flags` in the test runner, which recovered
/// graphics/shapes/server/web/moros_render/moros_sim).  Tracked under @P321.
const LIB_PKGS_NATIVE_SKIP: &[&str] = &[
    // (empty) An entry here names a package whose tests do not compile or run
    // under `--native`, the open issue that explains it, and the condition that
    // removes it.  Every package listed before ran green again by 2026-06.
];

/// Specific library test FILES skipped under `--native` (the rest of the
/// package DOES compile), keyed `"<pkg>/<file>.loft"`.
const LIB_TESTS_NATIVE_SKIP: &[&str] = &[
    // (empty) The `web` fixture's network tests live in `tests-network/`, which no
    // suite walks (TESTING.md § Every skip says why, how it runs instead, and when
    // it ends) — nothing to list.  An entry here needs the open issue that explains
    // it and the condition that removes it.
];

/// True if `entry` (a `lib/<pkg>/tests/<file>.loft` path) is skipped under the
/// NATIVE library gate (its own codegen-gap list above; the interpreter gate's
/// skips live in `wrap.rs::lib_test_skipped`).
fn native_lib_test_skipped(entry: &Path) -> bool {
    let file = entry
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let pkg = entry
        .parent()
        .and_then(|d| d.parent())
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if LIB_PKGS_NATIVE_SKIP.contains(&pkg.as_str()) {
        return true;
    }
    let key = format!("{pkg}/{file}");
    LIB_TESTS_NATIVE_SKIP.contains(&key.as_str())
}

/// Native counterpart of `wrap.rs::library_suite`: compile + run every
/// Run `loft [extra_args] test <stem>` for a lib package in a UNIQUE temp CWD —
/// a `.loft_test_tmp_*` SIBLING inside `lib/` (so the package's relative deps
/// `../<name>` still resolve to the real packages), with the package's contents
/// symlinked in.  Isolates cwd-relative test artifacts so the native and
/// interpreter lib suites (separate, concurrently-run test binaries) don't race
/// on a shared file (e.g. `moros_render_test.glb`).  Duplicated from `wrap.rs`
/// (integration-test binaries can't share fns).  Non-unix falls back to `pkg_dir`.
fn run_lib_test_in_temp_cwd(
    loft_bin: &str,
    pkg_dir: &Path,
    stem: &str,
    extra_args: &[&str],
) -> std::io::Result<std::process::Output> {
    let mut args: Vec<&str> = extra_args.to_vec();
    args.push("test");
    args.push(stem);
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let lib_root = pkg_dir.parent().unwrap_or(pkg_dir);
        let tmp = lib_root.join(format!(
            ".loft_test_tmp_{}_{}",
            std::process::id(),
            CTR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir(&tmp)?;
        for entry in std::fs::read_dir(pkg_dir)?.filter_map(|e| e.ok()) {
            let target = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            let _ = std::os::unix::fs::symlink(&target, tmp.join(entry.file_name()));
        }
        let out = std::process::Command::new(loft_bin)
            .current_dir(&tmp)
            .args(&args)
            .output();
        let _ = std::fs::remove_dir_all(&tmp);
        out
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(loft_bin)
            .current_dir(pkg_dir)
            .args(&args)
            .output()
    }
}

/// loft#878 — a test file that names a helper the way its LIBRARY does must still
/// compile under `--native`.
///
/// Emitted Rust is one flat namespace, so two same-named fns from different files get a
/// file-hash suffix on the DEFINITION (`disambiguated_fn_ident`, #305).  One call
/// emitter re-derived the identifier from `callee.name()` instead of going through
/// `Output::fn_ident`, so the call named a `fn` that had been emitted under another —
/// `error[E0425]: cannot find function n_defaulted in this scope`, on a package whose
/// interpreter suite was green.
///
/// The shape is narrow, which is why the reporter's own minimisation came out green and
/// is recorded here rather than repeated: it needs the colliding call to take the
/// ADOPT-or-COPY bind (`{ let _dst = …; let _src = <callee>(cell, …)`), which is the one
/// emitter that bypassed the chokepoint.  A first bind of a call result goes through
/// `calls.rs` and was always right.  Here the callee returns a LOCAL bound from another
/// call, which is what puts the caller's assignment on the adopt path.
///
/// Both halves are asserted: the library's private `defaulted(W) -> boolean` is REACHED
/// (through `cell`, which the test calls), and the test-local `defaulted(integer) -> W`
/// is reached directly — so the guard fails if either definition goes missing, not only
/// if the name resolves wrongly.
// @speed 1.2
#[test]
fn a_test_local_name_shadowing_a_library_fn_compiles_natively_878() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("shadowed-fn-name guard: skipped (no libloft.rlib / rustc unavailable)");
        return Ok(());
    }
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_878_{pid}"));
    let _ = std::fs::remove_dir_all(&root);
    let pkg = root.join("shadowlib");
    std::fs::create_dir_all(pkg.join("src"))?;
    std::fs::create_dir_all(pkg.join("tests"))?;
    std::fs::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"shadowlib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/shadowlib.loft\"\n",
    )?;
    std::fs::write(
        pkg.join("src/shadowlib.loft"),
        "pub struct W { w_n: integer, w_tag: text }\n\
         pub struct H { h_q: integer, h_r: integer }\n\
         pub fn make(n: integer) -> W { W { w_n: n, w_tag: \"lib\" } }\n\
         pub fn bump(w: W, n: integer) -> integer { w.w_n + n }\n\
         fn defaulted(w: W) -> boolean { w.w_n > 0 }\n\
         pub fn cell(w: W, q: integer) -> H {\n\
         \x20 if q < 0 {\n\
         \x20   if defaulted(w) { return H { h_q: w.w_n, h_r: 0 }; }\n\
         \x20   return H {};\n\
         \x20 }\n\
         \x20 return H { h_q: q, h_r: 1 };\n\
         }\n",
    )?;
    std::fs::write(
        pkg.join("tests/probe.loft"),
        "use shadowlib;\n\
         fn defaulted(h: integer) -> W {\n\
         \x20 w = make(h);\n\
         \x20 assert(bump(w, 0) == h, \"the fixture was built wrong: {w.w_n}\");\n\
         \x20 w\n\
         }\n\
         fn test_shadow() {\n\
         \x20 d = defaulted(3);\n\
         \x20 assert(d.w_n == 3, \"the test-local one answered {d.w_n}\");\n\
         \x20 c = cell(W { w_n: 5, w_tag: \"x\" }, -1);\n\
         \x20 assert(c.h_q == 5, \"the library reached its own private helper: {c.h_q}\");\n\
         \x20 e = cell(W { w_n: 0, w_tag: \"x\" }, -1);\n\
         \x20 assert(e.h_q == 0, \"and the helper answered false: {e.h_q}\");\n\
         }\n",
    )?;
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    for extra in [vec![], vec!["--native"]] {
        let out = run_lib_test_in_temp_cwd(loft_bin, &pkg, "probe", &extra)?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            combined.contains("test result: ok")
                && !combined.contains("native compile:")
                && !combined.contains("test result: FAILED"),
            "loft test {extra:?} on a package whose test file shadows a library fn name:\n{combined}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// `lib/<pkg>/tests/*.loft` under `--native`, skipping packages/files with known
/// native-codegen gaps (`LIB_*_NATIVE_SKIP`, @P321).  Shells out
/// `cd lib/<pkg> && loft --native test <stem>` so it reuses the CLI's package
/// resolution AND the `#native`-crate linkage (`add_native_extern_flags`).
///
/// Holds `native_suite_lock` so it serialises with the other native suites
/// (shared `/tmp` rlib + binary cache).  Skips silently when `rustc` / the loft
/// rlib are unavailable, like `native_scripts`.
// @speed 2.5
#[test]
fn native_library_suite() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("native_library_suite: skipped (no libloft.rlib / rustc unavailable)");
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let mut files: Vec<PathBuf> = Vec::new();
    for pkg in std::fs::read_dir("lib")?.filter_map(|e| e.ok()) {
        // Skip the `.loft_test_tmp_*` artifact-isolation dirs (see
        // run_lib_test_in_temp_cwd) so they're never discovered as packages.
        if pkg.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let tests_dir = pkg.path().join("tests");
        if !tests_dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&tests_dir)?.filter_map(|e| e.ok()) {
            let p = f.path();
            if p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0;
    let mut env_skips: Vec<(String, String)> = Vec::new();
    for entry in files {
        if native_lib_test_skipped(&entry) {
            println!("skip {entry:?} (LIB_*_NATIVE_SKIP — @P321)");
            continue;
        }
        let pkg_dir = entry.parent().and_then(|d| d.parent()).unwrap_or(&entry);
        let stem = entry
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        println!("native lib test {entry:?}");
        let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, &stem, &["--native"])?;
        ran += 1;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // Windows: the windows-targets crate emits a search path that
        // doesn't survive the test-binary link step (LNK1181: cannot
        // open input file 'windows.0.NN.0.lib').  Environmental, not a
        // code regression — mirrors the toolchain-skip pattern in
        // tests/exit_codes.rs.  loft test wraps the rustc error as
        // "native compile: error: linking with `link.exe` failed: exit
        // code: 1181" — the raw "LNK1181" symbol from cc's separate
        // stderr may not survive the capture.  Match both forms.
        if combined.contains("LNK1181") || combined.contains("link.exe` failed: exit code: 1181") {
            // Capture the actual linker error lines, not just the generic
            // label — without this the skip is a SILENT green (nextest hides
            // a passing test's stdout), so a regression of G2's fix would
            // look identical to a clean pass.  The detail is surfaced by the
            // CI "Surface environmental test skips" step via the ledger below.
            let detail: Vec<&str> = combined
                .lines()
                .filter(|l| l.contains("1181") || l.contains("LNK") || l.contains("link.exe"))
                .take(3)
                .collect();
            println!("skip {entry:?} (Windows windows-targets LNK1181 — environmental)");
            env_skips.push((format!("{entry:?}"), detail.join(" | ")));
            ran -= 1;
            continue;
        }
        // `loft test` exits 0 even on a caught crash; detect failure by markers.
        let failed = !out.status.success()
            || combined.contains("SIGSEGV")
            || combined.contains("panicked")
            || combined.contains("native compile:")
            || combined.contains("test result: FAILED")
            || !combined.contains("test result: ok");
        if failed {
            let tail: Vec<&str> = combined.lines().rev().take(5).collect();
            failures.push(format!("{entry:?}: {}", tail.join(" | ")));
        }
    }
    if !failures.is_empty() {
        return Err(Error::other(format!(
            "{} of {ran} native library tests failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )));
    }
    if !env_skips.is_empty() {
        common::record_env_skips("native_library_suite", "LNK1181", &env_skips);
        println!(
            "native_library_suite: {ran} passed, {} skipped (environmental — LNK1181)",
            env_skips.len()
        );
    } else {
        println!("native_library_suite: {ran} native library tests passed");
    }
    Ok(())
}

/// @PLN26 — the C-ABI native-package EXEC path: a program that `use`s a
/// `[native] crate` package, compiled with `--native`, must link the package's
/// cdylib by C-ABI (`add_native_extern_flags`) and call its `#native` symbol.
/// The minimal `native_scalar_pkg` fixture exports one scalar symbol
/// (`n_native_answer` → 42), so this is cheap to build yet covers the whole
/// path — including, on Windows, the import-library link + DLL staging of
/// @PLN26 phase 4 (the focused `win-cdylib` workflow's C-ABI job sets
/// `LOFT_NATIVE_CABI=1` to force that arm on; on the normal Windows CI this
/// exercises the rlib-link path instead).  The hard-coded 42 is the oracle, so
/// a broken link (undefined symbol / P269 / wrong value) fails LOUDLY — not
/// vacuously.  This is the regression guard the C-ABI exec path previously
/// lacked (phase 0 was a manual probe).
///
/// Serialises via `native_suite_lock` (shared /tmp rlib + binary cache) and
/// skips cleanly when `rustc` / the loft rlib are unavailable, like the other
/// native suites.
#[test]
fn native_crate_package_links_and_runs_via_cabi() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("native_crate_package_links_and_runs_via_cabi: skipped (no libloft.rlib / rustc)");
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let pkg_dir = Path::new("tests/lib/native_scalar_pkg");

    // --native: the C-ABI exec path (the path @PLN26 phase 4's Windows arm extends).
    let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "answer", &["--native"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("no implementation"),
        "native_crate package symbol was not linked (P269) — the C-ABI exec path regressed:\n{combined}"
    );
    assert!(
        combined.contains("test result: ok"),
        "--native native_crate exec test did not pass:\n{combined}"
    );

    // --interpret parity: the same package via the runtime dylib dispatch.
    let out_i = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "answer", &["--interpret"])?;
    let combined_i = format!(
        "{}{}",
        String::from_utf8_lossy(&out_i.stdout),
        String::from_utf8_lossy(&out_i.stderr)
    );
    assert!(
        combined_i.contains("test result: ok"),
        "--interpret native_crate exec test did not pass:\n{combined_i}"
    );
    Ok(())
}

/// loft#907 — a `#native "sym"` its library implements under a DIFFERENT Rust
/// name must reach that implementation on BOTH backends.
///
/// `#native "sym"` is an API id: a library registers its implementations by loft
/// symbol (`loft_register_bridges! { "sym" => other__loft_bridge }`) and may point
/// one at a differently-named fn.  `--interpret` reads that table; `--native` put
/// the `#native` string straight into a `#[link_name]`, so it bound whatever else
/// the cdylib exported under that name.  In the published `graphics` that was the
/// same call in the older raw `(ptr, count)` shape rather than loft's
/// `(LoftStore, LoftRef)` one, for ten functions — `save_png` returned `false`
/// under `--native` and `true` under `--interpret`, silently, and the WebGL
/// upload calls were mis-marshalled the same way.
///
/// The `native_remap_pkg` fixture exports a DECOY under each `#native` name
/// (-1000 / -2000), so a regression does not merely fail to link — it answers,
/// and the answer names which resolution path was taken.  `native_scalar_pkg`
/// above is the clean-binding control: the redirect must leave it untouched.
///
/// Serialises via `native_suite_lock` and skips cleanly without `rustc` / the
/// loft rlib, like the other native suites.
#[test]
fn remapped_native_symbol_resolves_to_its_implementation_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!(
            "remapped_native_symbol_resolves_to_its_implementation_on_both_backends: \
             skipped (no libloft.rlib / rustc)"
        );
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let pkg_dir = Path::new("tests/lib/native_remap_pkg");

    for backend in ["--native", "--interpret"] {
        let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "remap", &[backend])?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains("no implementation"),
            "{backend}: the remapped native symbol was not linked at all (P269):\n{combined}"
        );
        assert!(
            combined.contains("test result: ok"),
            "{backend}: a remapped `#native` symbol did not reach its implementation \
             (loft#907) — the assertion message names the wrong answer it got:\n{combined}"
        );
    }
    Ok(())
}

/// The imaging fixture's PNG round-trip — a `[native] crate` package whose
/// `#native` functions (`load_png`/`save_png`) do STORE-MUTATING file I/O via
/// raw `std::fs`.  Guards two things at once: (1) the store-mutating C-ABI path
/// (LoftStore + LoftRef marshalling, the hardest native shape), and (2) the
/// source-dir cwd anchoring — `loft test` runs from the package root, but a
/// native crate's `std::fs` must resolve `map.png` / `_tmp_*.png` where loft's
/// `file()` does (`tests/`), which only holds because the runner chdir's to
/// `source_dir`.  Both round-trip files assert hard pixel oracles, so a broken
/// link OR a cwd regression fails loudly.  Runs on BOTH backends.
///
/// Serialises via `native_suite_lock`; skips cleanly without `rustc` / the loft
/// rlib, like the other native suites.
#[test]
fn imaging_fixture_png_roundtrip_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("imaging_fixture_png_roundtrip_both_backends: skipped (no libloft.rlib / rustc)");
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let pkg_dir = Path::new("tests/fixtures/libs/imaging");
    for stem in ["14-image", "15-regression"] {
        for mode in ["--native", "--interpret"] {
            let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, stem, &[mode])?;
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                combined.contains("test result: ok"),
                "imaging {stem} {mode} round-trip did not pass:\n{combined}"
            );
        }
    }
    Ok(())
}

/// loft#742 — a NESTED vector field must reference the content type the
/// compiler recorded, not one rebuilt from the loft `Type`.
///
/// `type_def_nr` cannot see an element's `forced_size`, so rebuilding the
/// nesting produced `db.vector(db.vector(<plain integer>))` for BOTH
/// `vector<vector<integer>>` and `vector<vector<integer(-32768, 32767)>>` —
/// the wrong element width, and a `vector<vector<integer>>` minted at an id
/// the compiler had assigned to something else, which shifted every runtime
/// type id after it.
///
/// The check is `LOFT_STRICT_SCHEMA_IDS`, which turns the `init()` schema
/// comparison into a hard failure at the first divergence — the same gate that
/// found this. Asserting the program's OUTPUT alone would not catch it: these
/// programs print correct answers today, because nothing happened to bake an
/// id at or past the shift.
#[test]
fn a_nested_narrow_vector_field_keeps_the_type_ids_aligned() {
    if std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skip: rustc unavailable (--native needs it)");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Each of these carries a differently-shaped nested narrow vector, and each
    // drifted by a different amount before the fix.
    //
    // loft#923 added the fourth: a `vector<τ?>` ELEMENT drifted for the same
    // reason one level up. The emitter tested the element type without peeling
    // its `Optional`, so a `vector<vector<integer>?>` missed the nested-vector
    // arm and the generic path minted a type the program never named. It belongs
    // in THIS list rather than beside its own script, because a drift is invisible
    // to the program's output and only this env makes it fail.
    for script in [
        "tests/scripts/184-nested-narrow-int-vector.loft",
        "tests/scripts/624-nested-narrow-width.loft",
        "tests/scripts/432-untyped-vector-literal-arg.loft",
        "tests/scripts/923-nullable-vector-element-schema.loft",
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--native")
            .arg(root.join(script))
            .env("LOFT_STRICT_SCHEMA_IDS", "1")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("diverges from the compiler"),
            "{script}: the generated schema drifted from the compiler's.\n{stderr}"
        );
        assert!(
            out.status.success(),
            "{script}: exited non-zero.\n{}\n{stderr}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

/// loft#1311 — a FN-LEVEL `@EXPECT_FAIL` excuses one function, not the whole file.
///
/// Two defects met here.  The suite dropped the file for ANY declaration of the tag, and
/// the finer per-function mechanism underneath it — which emits
/// `// skipped (EXPECT_FAIL): {name}` in place of a call — could not parse the documented
/// `// @EXPECT_FAIL: <reason>` form, because its hand-rolled `split_whitespace` looked for
/// the bare token and the documented one carries a colon.  So the skip-set was empty for
/// every file written the documented way, and the drop above it silently cost each of
/// those files' passing functions their native coverage.
///
/// The guard reads the GENERATED main, which is where the contract is decidable: the
/// excused function must appear as a skip, and every sibling must still be called.
/// Asserting only that the file was prepared would pass with the skip-set still empty.
#[test]
fn a_fn_level_expect_fail_keeps_its_siblings_in_the_native_suite() {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());

    // The documented colon form is the one the old parser could not read.
    let src = "\
fn test_1311_sibling_before() { assert(1 == 1, \"before\"); }

// @EXPECT_FAIL: deliberately broken
fn test_1311_excused() { assert(false, \"deliberate\"); }

fn test_1311_sibling_after() { assert(2 == 2, \"after\"); }
";

    // Parsed alone, the tag names one function and declares nothing file-level.
    let (fns, file_level) = common::expect_fail_fns(src);
    assert!(
        fns.contains("test_1311_excused"),
        "the documented `@EXPECT_FAIL: <reason>` form must name its fn; got {fns:?}"
    );
    assert!(
        !file_level,
        "a tag under a declaration is fn-level, not file-level"
    );
    // Control: the SAME tag in the header IS file-level, so the drop still has a subject.
    let (_, header_level) = common::expect_fail_fns("// @EXPECT_FAIL: whole file\nfn t() { }\n");
    assert!(header_level, "a header tag must still read as file-level");

    let scratch = loft::platform::scratch_dir();
    let entry = scratch.join("loft1311_fn_level_expect_fail.loft");
    std::fs::write(&entry, src).expect("write the probe script");

    let job = prepare_native_test(&entry).expect("a fn-level @EXPECT_FAIL must still prepare");
    let generated = std::fs::read_to_string(&job.tmp_rs).expect("read the generated Rust");

    assert!(
        generated.contains("// skipped (EXPECT_FAIL): n_test_1311_excused"),
        "the excused fn must be skipped, not called:\n{generated}"
    );
    for sibling in ["n_test_1311_sibling_before", "n_test_1311_sibling_after"] {
        assert!(
            generated.contains(&format!("{sibling}(&cell)")),
            "sibling {sibling} lost its native coverage:\n{generated}"
        );
    }

    let _ = std::fs::remove_file(&entry);
}
