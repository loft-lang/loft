// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a library's compiled artifact, adopted or built

//! The driver's one question about a library this run: adopt its compiled artifact, build
//! one, or interpret it.  The interpreter answers the first half
//! (`native_adopt::adopt_shared_cdylib`) and the Rust writer builds
//! (`native_lib::build_shared_artifact`); neither names the other, so the two meet here.

use crate::data::Data;
use crate::database::Stores;
use crate::interp::native_adopt::Adopt;
use std::collections::HashSet;

/// @PLN11 Arc N / N3 (Step 4 — **dev-interpret-on-edit**) — decide how the library
/// at `pkg_dir` should run this invocation, and build its cdylib only when warranted.
///
/// Returns:
/// - `Ok(Some(so))` — dispatch native against the cdylib at `so` (fresh cached, or
///   just built);
/// - `Ok(None)` — **interpret this run** (the library is being actively edited);
/// - `Err(_)` — a build was attempted and failed (the caller interprets + warns).
///
/// The policy reconciles "a library is always native" with "no `rustc` per save":
/// 1. **Fresh artifact** (`.so` exists, fingerprint matches, source not newer) →
///    native, no work.
/// 2. **No artifact yet, or `loft` itself changed** (fingerprint mismatch) → **build
///    eagerly** → native.  First use of a library, and a deployed/stable dep, pay the
///    one-time build and run native from the start (this is also why the parity tests,
///    which wipe `native-auto/` then run once, still dispatch native).
/// 3. **Stale artifact** (an `.so` exists but the source is newer — the library was
///    edited) → consult the last-run source hash:
///    - *changed since the previous run* → the edit loop is live → **interpret this
///      run** (`Ok(None)`); record the new hash.  No `rustc`.
///    - *unchanged since the previous run* → editing has settled → **rebuild** → native.
///
/// So an edit-run-edit-run loop interprets each time (no `rustc`); the first run after
/// you stop editing rebuilds and the library is native again.  (Polish: do that
/// rebuild in the background so even the settling run never blocks — see the plan's
/// Step 4 option 3.)
///
/// # Errors
/// Propagates a build failure from [`build_shared_cdylib`](crate::native_lib::build_shared_cdylib).
pub fn cached_or_build_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
    pending: &[String],
    context: &LibraryContext,
) -> Result<Option<std::path::PathBuf>, String> {
    match crate::interp::native_adopt::adopt_shared_cdylib(
        data, stores, export_set, pkg_dir, pending,
    )? {
        Adopt::Fresh(so) => Ok(Some(so)),
        Adopt::Interpret => Ok(None),
        // `@PLN206 P4` — a hosted C unit, which works on this interpreter's stores, instead of
        // a rustc artifact (`LOFT_LIB_BACKEND=c`).
        Adopt::Build(key) if crate::c::hosted::selected() => {
            let mut alone = parse_library_alone(
                pkg_dir,
                &context.default_dir,
                &context.lib_dirs,
                context.scope_entry.as_deref(),
            )?;
            let export = alone.export.clone();
            let p = &mut alone.parser;
            crate::c::hosted::build_artifact(
                &mut p.data,
                &mut p.database,
                &export,
                &alone.pkg_dir,
                &key.out_dir,
                &key.stem,
                key.fp,
            )
            .map(Some)
        }
        Adopt::Build(key) => crate::native_lib::build_shared_artifact(
            data,
            stores,
            export_set,
            pkg_dir,
            &key.out_dir,
            &key.stem,
            key.fp,
        )
        .map(Some),
    }
}

/// Where a library's own build finds what it is parsed with: the standard library's directory
/// (or none, for the one this binary embeds) and the `use` search path of the run.
#[derive(Default)]
pub struct LibraryContext {
    pub default_dir: String,
    pub lib_dirs: Vec<String>,
    /// The program's entry file, whose resolution scope (its lock pins) the library's own
    /// dependencies resolve by, as they did in the program; `None` for `build-native`.
    pub scope_entry: Option<String>,
}

/// The spelling of `pkg` (canonical) the parse loaded the package's sources by, when it is
/// another than `pkg` itself.
fn loaded_spelling(data: &crate::data::Data, pkg: &std::path::Path) -> Option<String> {
    let pkg_text = pkg.to_string_lossy();
    let mut seen = std::collections::HashSet::new();
    for d in 0..data.definitions() {
        let file = data.def(d).position().file.as_str();
        if !seen.insert(file) {
            continue;
        }
        if crate::file_access::is_under(file, &pkg_text) {
            return None;
        }
        for dir in std::path::Path::new(file).ancestors().skip(1) {
            if dir.as_os_str().is_empty() {
                break;
            }
            if crate::file_access::plain_canonical(dir) == pkg {
                return Some(dir.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// A library parsed alone: the program `use <name>;` over the standard library, the way every
/// program that uses the library sees its definitions.  What a library's own artifact is built
/// from (`loft build-native`, a hosted C unit — `@PLN206 P5`), so the artifact is the library's
/// and not the program's that happened to build it first.
pub struct LibraryAlone {
    pub parser: crate::parser::Parser,
    /// The package directory, canonical: what `native_lib::library_export_set` filters by.
    pub pkg_dir: String,
    /// The library's public functions an artifact carries.
    pub export: HashSet<u32>,
    /// The libraries compiled with it (its dependencies), as a program's run lists them.
    pub pending: Vec<String>,
}

/// Parse the library at `pkg_dir` alone, its dependencies resolved through `lib_dirs` and the
/// package's own parent directory (before the registry), over the standard library at
/// `default_dir` (or the one this binary embeds), with the slot and scope analysis a writer
/// reads.
///
/// # Errors
/// The package has no name, the standard library does not load, or the library does not parse.
pub fn parse_library_alone(
    pkg_dir: &str,
    default_dir: &str,
    lib_dirs: &[String],
    scope_entry: Option<&str>,
) -> Result<LibraryAlone, String> {
    let pkg_path = crate::file_access::plain_canonical(std::path::Path::new(pkg_dir));
    let pkg_dir = pkg_path.to_string_lossy().to_string();
    // The name `use` finds it by: its `[package] name`, else its directory's, as the
    // interpreter resolves a library with no `[package]` section.
    let manifest = crate::manifest::read_manifest(&pkg_path.join("loft.toml").to_string_lossy());
    let name = manifest
        .as_ref()
        .and_then(|m| m.name.clone())
        .or_else(|| crate::file_access::file_name(&pkg_path))
        .ok_or_else(|| format!("{pkg_dir} is not a loft package (no [package] name)"))?;
    // What it declares it depends on is loaded with it, as in any program that loads it: a
    // `[c] shim` may include a dependency's header that no `use` of its loft names.
    let deps: Vec<String> = manifest
        .map(|m| m.dependencies.into_iter().map(|(d, _)| d).collect())
        .unwrap_or_default();
    let mut p = crate::parser::Parser::new();
    // The package's own parent first: `use <name>` must find THIS checkout before a
    // same-named registry package, or `library_export_set` (by `pkg_dir` prefix) sees none.
    if let Some(parent) = pkg_path.parent() {
        p.lib_dirs.push(parent.to_string_lossy().to_string());
    }
    for d in lib_dirs {
        if !p.lib_dirs.contains(d) {
            p.lib_dirs.push(d.clone());
        }
    }
    // The library's dependencies resolve as the program's did: by its lock pins, not the newest
    // release — the unit carries their code, and the artifact's key names the program's copies.
    if let Some(entry) = scope_entry {
        p.govern_by(entry);
    }
    p.parse_stdlib(default_dir)
        .map_err(|e| format!("the standard library: {e}"))?;
    let tmp = std::env::temp_dir().join(format!(
        "loft_library_alone_{}_{}.loft",
        std::process::id(),
        name
    ));
    let mut program = format!("use {name};\n");
    for d in &deps {
        program.push_str(&format!("use {d};\n"));
    }
    crate::file_access::write(&tmp, program)
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    p.parse(&tmp.to_string_lossy(), false);
    let _ = crate::file_access::remove_file(&tmp);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        let first = p
            .diagnostics
            .lines()
            .into_iter()
            .find(|l| l.contains("rror"))
            .unwrap_or_default();
        return Err(format!("`{name}` does not parse alone: {first}"));
    }
    // The slot and scope analysis the writers depend on, as the run path does before its build.
    crate::scopes::check(&mut p.data, &mut p.database);
    // The package as the parse spelled it: a registry library resolves through `LOFT_HOME`,
    // which may name the canonical directory another way (`/var` for `/private/var`), and the
    // export set is read by path prefix.
    let pkg_dir = loaded_spelling(&p.data, &pkg_path).unwrap_or(pkg_dir);
    let export = crate::native_lib::library_export_set(&p.data, &pkg_dir);
    let mut pending = std::mem::take(&mut p.pending_native_compile);
    if !pending.iter().any(|d| d == &pkg_dir) {
        pending.push(pkg_dir.clone());
    }
    // The natives it calls resolved to their implementations, as the run path does
    // (`extensions::preload_native_libs`).
    let hand_written = p.pending_native_libs.clone();
    crate::interp::extensions::preload_native_libs(&mut p.data, &hand_written);
    Ok(LibraryAlone {
        parser: p,
        pkg_dir,
        export,
        pending,
    })
}
