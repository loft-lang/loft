// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a library's compiled artifact, adopted or built

//! The driver's one question about a library this run: adopt its compiled artifact, build
//! one, or interpret it.  The interpreter answers the first half
//! (`native_adopt::adopt_shared_cdylib`) and the C backend builds a hosted unit
//! (`c::hosted::build_artifact`); neither names the other, so the two meet here.

use std::fmt::Write as _;

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
/// Propagates a build failure from [`build_artifact`](crate::c::hosted::build_artifact).
pub fn cached_or_build_shared_cdylib(
    data: &Data,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
    pending: &[String],
    context: &LibraryContext,
) -> Result<Option<std::path::PathBuf>, String> {
    match crate::interp::native_adopt::adopt_shared_cdylib(data, export_set, pkg_dir, pending)? {
        Adopt::Fresh(so) => Ok(Some(so)),
        Adopt::Interpret => Ok(None),
        // `@PLN206` — a library's compiled form is a hosted C unit, which works on this
        // interpreter's stores; no rustc takes part.
        Adopt::Build(key) => {
            let mut alone = parse_library_alone(
                pkg_dir,
                &context.default_dir,
                &context.lib_dirs,
                &context.flag_lib_dirs,
                context.scope_entry.as_deref(),
            )?;
            let export = alone.export.clone();
            let p = &mut alone.parser;
            let built = crate::c::hosted::build_artifact(
                &mut p.data,
                &mut p.database,
                &export,
                &alone.pkg_dir,
                &key.out_dir,
                &key.stem,
                key.fp,
            )?;
            crate::cache::write_native_artifact_fingerprint(&key.out_dir, key.fp);
            // Bound the directory while the build lock is still held: a unit appears per key
            // (a loft, a switch), and nothing else collects the old ones.
            prune_artifacts(
                &key.out_dir,
                &crate::opt::native_key::auto_cdylib_stem(pkg_dir),
            );
            Ok(Some(built))
        }
    }
}

/// Where a library's own build finds what it is parsed with: the standard library's directory
/// (or none, for the one this binary embeds) and the `use` search path of the run.
#[derive(Default)]
pub struct LibraryContext {
    pub default_dir: String,
    pub lib_dirs: Vec<String>,
    /// The `--lib` directories of the run (`@C173`), which outrank a pin there as here.
    pub flag_lib_dirs: Vec<String>,
    /// The program's entry file, whose resolution scope (its lock pins) the library's own
    /// dependencies resolve by, as they did in the program; `None` for `build-native`.
    pub scope_entry: Option<String>,
}

/// `@PLN206 P7a` — adopt the compiled units of the libraries `parser` loaded, for a session that
/// runs the program (the debugger): the steps a plain run takes per library
/// (`native_adopt::scope_library`, the export set and `hosted_split`,
/// `cached_or_build_shared_cdylib`, `probe_and_mark_exports`), each adopted artifact queued for
/// `extensions::load_all`.  A library that does not build interprets, as it did, with nothing
/// said: a debug session asked for no build.  `LOFT_NO_NATIVE_LIBS` keeps every library
/// interpreted.
pub fn adopt_for_session(parser: &mut crate::parser::Parser, entry: &str, default_dir: &str) {
    let pending = std::mem::take(&mut parser.pending_native_compile);
    if pending.is_empty() || std::env::var_os("LOFT_NO_NATIVE_LIBS").is_some() {
        return;
    }
    let hand_written = parser.pending_native_libs.clone();
    crate::interp::extensions::preload_native_libs(&mut parser.data, &hand_written);
    let context = LibraryContext {
        default_dir: default_dir.to_string(),
        lib_dirs: parser.lib_dirs.clone(),
        flag_lib_dirs: parser.flag_lib_dirs.clone(),
        scope_entry: Some(entry.to_string()),
    };
    for pkg_dir in &pending {
        if crate::file_access::is_under(entry, pkg_dir)
            || crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
                .is_some_and(|m| m.native_in_binary)
        {
            continue;
        }
        let export = crate::opt::native_key::library_export_set(&parser.data, pkg_dir);
        let export = crate::c::hosted::hosted_split(&parser.data, &parser.database, &export).0;
        if export.is_empty() {
            continue;
        }
        let built =
            cached_or_build_shared_cdylib(&parser.data, &export, pkg_dir, &pending, &context);
        if let Ok(Some(so)) = built {
            let probe = crate::interp::native_adopt::probe_and_mark_exports(
                &mut parser.data,
                &parser.database,
                &export,
                &so,
            );
            if probe.marked > 0 {
                parser
                    .pending_native_libs
                    .push(so.to_string_lossy().into_owned());
            }
        }
    }
}

/// `@PLN206 P7` — `loft debug --native`: the program's own functions, `main` aside, built as one
/// hosted C unit and adopted, so the debugged program runs compiled — `main` the interpreter's
/// driver, every function it calls C.  A routine the debugger wants (a breakpoint in it or
/// reachable from it, a step into it) runs interpreted as a library's does, and a pause while a
/// compiled routine runs hands it to the interpreter at its next loop head.  The unit is built
/// from the program parsed again as the session parsed it (`Parser::open_program`, then `path`
/// as spelled — the generation needs its own `Data`, and the bridge names and keys must be the
/// session's), under a key of this loft and every source the program loaded, in the user cache
/// (`debug-units/`).  Answers how many functions now run compiled.
///
/// # Errors
/// When the program does not parse alone, or the unit does not build.
pub fn adopt_program_for_session(
    parser: &mut crate::parser::Parser,
    path: &str,
    default_dir: &str,
) -> Result<usize, String> {
    use crate::file_access as fa;
    let default_dir_given = default_dir;
    let entry = fa::plain_canonical(std::path::Path::new(path));
    let program_dir = entry
        .parent()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    let default_dir = if default_dir.is_empty() {
        String::new()
    } else {
        fa::plain_canonical(std::path::Path::new(default_dir))
            .to_string_lossy()
            .into_owned()
    };
    let export_of = |data: &Data, stores: &Stores| -> HashSet<u32> {
        let own = program_functions(data, &program_dir, &default_dir);
        let gate = crate::native_gate::shared_store_dispatchable(data);
        let open: HashSet<u32> = gate.intersection(&own).copied().collect();
        crate::c::hosted::hosted_split(data, stores, &open).0
    };
    let export = export_of(&parser.data, &parser.database);
    if export.is_empty() {
        return Ok(0);
    }
    let fp = program_unit_key(&parser.data);
    let out_dir = crate::cache::cache_base_dir().join("debug-units");
    let stem = format!("loft_debug_{fp:016x}");
    let so = out_dir.join(crate::opt::native_key::platform_cdylib_name(&stem));
    if !fa::exists(&so) {
        fa::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
        let lock_path = out_dir.join(format!("{stem}.lock"));
        let lock = fa::create(&lock_path).map_err(|e| format!("create {e}"))?;
        lock.lock()
            .map_err(|e| format!("lock {}: {e}", lock_path.display()))?;
        if !fa::exists(&so) {
            let src = fa::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
            let mut p = crate::parser::Parser::open_program(default_dir_given, &parser.lib_dirs)
                .map_err(|e| format!("the standard library: {e}"))?;
            p.parse_str(&src, path, false);
            if p.diagnostics.level() >= crate::diagnostics::Level::Error {
                return Err("the program does not parse alone".into());
            }
            crate::scopes::check(&mut p.data, &mut p.database);
            let hand_written = p.pending_native_libs.clone();
            crate::interp::extensions::preload_native_libs(&mut p.data, &hand_written);
            let alone = export_of(&p.data, &p.database);
            crate::c::hosted::build_artifact(
                &mut p.data,
                &mut p.database,
                &alone,
                &program_dir,
                &out_dir,
                &stem,
                fp,
            )?;
        }
    }
    let probe = crate::interp::native_adopt::probe_and_mark_exports(
        &mut parser.data,
        &parser.database,
        &export,
        &so,
    );
    if let Some(missing) = probe.refused {
        return Err(format!(
            "the unit names `{missing}`, which this program has not"
        ));
    }
    if std::env::var_os("LOFT_TRACE_HANDOVER").is_some() {
        eprintln!(
            "[handover] the program's unit: {} of {} function(s) compiled; not resolved: {:?}",
            probe.marked,
            export.len(),
            probe.unresolved
        );
    }
    if probe.marked > 0 {
        parser
            .pending_native_libs
            .push(so.to_string_lossy().into_owned());
    }
    Ok(probe.marked)
}

/// The key of a program's unit: this loft, the switches that change a unit, and every source
/// the program loaded (by path and content).
fn program_unit_key(data: &Data) -> u64 {
    let mut files: Vec<String> = (0..data.definitions())
        .map(|d| data.def(d).position().file.as_str().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    files.sort_unstable();
    files.dedup();
    let mut parts: Vec<Vec<u8>> = vec![
        crate::cache::loft_build_fingerprint()
            .to_le_bytes()
            .to_vec(),
        crate::cache::loft_exe_identity().to_le_bytes().to_vec(),
        vec![
            u8::from(crate::keys::no_handover()),
            u8::from(crate::keys::poison_claim()),
            u8::from(crate::keys::hosted_planted()),
            u8::from(!crate::keys::exp_pow_enabled()),
        ],
    ];
    for f in &files {
        parts.push(f.as_bytes().to_vec());
        parts.push(crate::cache::file_hash(f).map_or_else(Vec::new, |h| h.to_vec()));
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    crate::cache::stable_hash(&refs)
}

/// The program's own functions a unit can carry: those written in the program's directory —
/// not the standard library's, nor a library's (an adopted one is already native) — and not
/// `main`, which the interpreter runs as the driver.  Directories are compared canonical, so a
/// program loaded by a relative path is found.
fn program_functions(data: &Data, program_dir: &str, default_dir: &str) -> HashSet<u32> {
    let mut own_file: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut is_own = |file: &str| -> bool {
        *own_file.entry(file.to_string()).or_insert_with(|| {
            let c = crate::file_access::plain_canonical(std::path::Path::new(file));
            let c = c.to_string_lossy();
            crate::file_access::is_under(&c, program_dir)
                && (default_dir.is_empty() || !crate::file_access::is_under(&c, default_dir))
        })
    };
    (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), crate::data::DefType::Function)
                && def.name() != "n_main"
                && !crate::opt::native_key::is_synthetic_name(def.name())
                && def.native().is_empty()
                && def.c_sig.is_empty()
                && !def.position().file.as_str().is_empty()
                && is_own(def.position().file.as_str())
        })
        .collect()
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
    /// The package directory, canonical: what `opt::native_key::library_export_set` filters by.
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
    flag_lib_dirs: &[String],
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
    // `@C173` — the run's `--lib` override reaches the library's own `use`s as it does the
    // program's, so the unit carries the copy the program loads.
    p.flag_lib_dirs = flag_lib_dirs.to_vec();
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
        let _ = writeln!(program, "use {d};");
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
    let export = crate::opt::native_key::library_export_set(&p.data, &pkg_dir);
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

/// How many built artifacts a package's `native-auto/` keeps.
///
/// The name carries the caller's type-layout fingerprint (#715), so a NEW file
/// appears whenever a consumer's type table differs — and nothing ever removed the
/// old ones.  Measured in this repo: `tests/lib/typeshift/native-auto` held 532
/// artifacts, 9.1 GB, at ~28 MB per suite run, because that fixture's whole job is
/// to shift type layouts.  Across the tree it was 25 GB.
///
/// Generous on purpose.  A pruned artifact that is wanted again is REBUILT, so the
/// cost of keeping too few is a rebuild and the cost of keeping too many is
/// disk; 8 covers a package used from several consumer contexts in one session
/// while still bounding the directory.
const KEEP_ARTIFACTS: usize = 8;

/// Drop all but the [`KEEP_ARTIFACTS`] most recent artifacts in `dir`, each with
/// the generated `.rs` / `.args` it was built from.
///
/// Called after a successful build, while the build lock is still held.  Ordering
/// is by mtime, so the survivors are the most recently BUILT — including the one
/// that just finished.
///
/// `family` is the artifact-name prefix this sweep OWNS
/// ([`auto_cdylib_stem`]) — and passing it is load-bearing, not tidiness.  The
/// sweep used to take every `.so` in the directory, and `native-auto/` is not
/// exclusively ours: a package with a `[c] shim` builds its shim cdylib
/// (`<pkg>_shim_<key>.so`) right there.  That library is content-keyed and built
/// ONCE, so it is always the OLDEST file in the directory — the first thing an
/// age-ordered sweep deletes the moment the directory saturates.  Deleting it
/// does not cost a rebuild, it removes the only definition of the package's `#c`
/// symbols, and the next run dies with *"`#c` symbol 'x' not found — check the
/// spelling"*, naming neither the library nor the sweep that ate it.  That is the
/// residual half of loft#831: parallel runs are what saturate the directory,
/// because each distinct type-layout context adds an artifact of its own.
///
/// A concurrent process can be adopting one of these without holding the lock (the
/// fresh fast path takes none).  That is why [`artifact_layout_probe`] refuses to
/// adopt an artifact it cannot OPEN: the loser of that race rebuilds instead of
/// dlopening a file that just vanished.
pub(crate) fn prune_artifacts(dir: &std::path::Path, family: &str) {
    let ext = crate::platform::dll_extension();
    let Ok(entries) = crate::file_access::read_dir(dir) else {
        return;
    };
    let mut built: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .into_iter()
        .filter(|e| crate::file_access::extension(e).is_some_and(|x| x == ext))
        .filter(|e| artifact_stem(e).is_some_and(|s| s.starts_with(family)))
        .filter_map(|e| {
            let m = crate::file_access::symlink_metadata(&e)
                .ok()?
                .modified()
                .ok()?;
            Some((m, e.os_spelling()))
        })
        .collect();
    if built.len() <= KEEP_ARTIFACTS {
        return;
    }
    // Newest first; everything past the keep window goes.
    built.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    for (_, so) in built.drain(KEEP_ARTIFACTS..) {
        let Some(stem) = artifact_stem(&so) else {
            continue;
        };
        let _ = crate::file_access::remove_file(&so);
        // The unit's C goes with it; `.rs` and `.args` are what older builds left beside one.
        for companion in ["c", "rs", "args"] {
            let _ = crate::file_access::remove_file(dir.join(format!("{stem}.{companion}")));
        }
    }
}

/// `libfoo_<fp>.so` / `foo_<fp>.dll` → the `foo_<fp>` the generated sources are
/// named after.  Only the platform prefix differs, and stripping it is what lets
/// one loop delete a library and its generated companions together — and what
/// lets the sweep tell one family of artifacts from another.
fn artifact_stem(so: impl crate::file_access::HostPath) -> Option<String> {
    let name = crate::file_access::file_stem(so)?;
    Some(
        name.strip_prefix("lib")
            .map_or_else(|| name.clone(), str::to_string),
    )
}
