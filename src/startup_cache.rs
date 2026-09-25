// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I86 — Startup cache & embedded stdlib

//! @PLN11 arc D / D2b — opt-in stdlib startup cache wiring.
//!
//! On a **warm** run (the `LOFT_STDLIB_CACHE` env var set and a valid bundle on
//! disk), [`warm_load_stdlib`] mmaps the precompiled stdlib bundle — the native
//! [`crate::data::Data`] **and** its database type schema — into the parser,
//! skipping the parse of `default/` entirely (~12× faster, see § arc D probe).
//! On a **cold** run, [`save_stdlib_cache`] writes that bundle after the parse.
//!
//! The cache file is keyed by [`crate::cache::stdlib_cache_key`] (stdlib
//! content, loft version, build id, target, feature set), so any stdlib edit
//! or toolchain bump lands at a fresh path and the stale bundle is simply
//! never read.  Default behaviour (env var unset) is unchanged: both functions
//! are no-ops, so normal runs always parse.

use crate::parser::Parser;

/// The cache-bundle path when the cache is enabled and the stdlib is readable;
/// `None` to disable (env var unset/empty, or `default/` unreadable).
#[cfg(feature = "mmap")]
fn cache_target(default_dir: &str) -> Option<std::path::PathBuf> {
    if std::env::var_os("LOFT_STDLIB_CACHE").is_none_or(|v| v.is_empty())
        && !crate::cache::program_cache_enabled()
    {
        return None;
    }
    let srcs = crate::cache::collect_stdlib_sources(default_dir);
    if srcs.is_empty() {
        return None;
    }
    Some(crate::cache::stdlib_cache_path(
        &crate::cache::stdlib_cache_key(&srcs),
    ))
}

/// Warm path: if the cache is enabled and a valid bundle exists, load the stdlib
/// `Data` + type schema into `p` and return `true` (the caller then skips
/// parsing `default/`).  Returns `false` to fall back to a cold parse.
#[cfg(feature = "mmap")]
#[must_use]
pub fn warm_load_stdlib(p: &mut Parser, default_dir: &str) -> bool {
    let Some(path) = cache_target(default_dir) else {
        return false;
    };
    match crate::ir_read::open_bundle_into(&path.to_string_lossy(), &mut p.database) {
        Ok(data) => {
            p.data = data;
            true
        }
        Err(_) => false, // cache miss (different key / absent) → cold parse
    }
}

/// Non-`mmap` builds (e.g. the wasm config) have no file-backed store, so the
/// cache is always a no-op and the caller always parses.
#[cfg(not(feature = "mmap"))]
#[must_use]
pub fn warm_load_stdlib(_p: &mut Parser, _default_dir: &str) -> bool {
    false
}

/// Cold path: after `default/` has been parsed into `p`, write the stdlib bundle
/// so the next run can warm-load it.  No-op when the cache is disabled.
#[cfg(feature = "mmap")]
pub fn save_stdlib_cache(p: &Parser, default_dir: &str) {
    let Some(path) = cache_target(default_dir) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = crate::ir_store::save_bundle(&p.data, &p.database.types, &path.to_string_lossy());
}

/// Non-`mmap` builds: no bundle to write.
#[cfg(not(feature = "mmap"))]
pub fn save_stdlib_cache(_p: &Parser, _default_dir: &str) {}

// ─── whole-program cache (arc E) ─────────────────────────────────────────────
//
// The stdlib cache above caches `default/` only; the whole-program cache caches
// the ENTIRE post-parse program (stdlib + the script's lazily-loaded libs + the
// user file) keyed on the script path, validated by a drift manifest of every
// parsed source's content hash.  On a repeated run of an unchanged program it
// skips ALL parsing.  The caller (`main.rs`) gates these on the cache env var;
// they assume the gate has already passed.

/// The stdlib a program bundle was built against, as the `stdk` line of its manifest:
/// the stdlib cache key over every `default/` file's name and content.  A program bundle
/// holds the parsed stdlib, and a program-cache miss may take the stdlib from its own cache,
/// so the bundle's source list need not name the stdlib files at all — this line is what
/// ties the bundle to the library it was parsed against, and an edited, added, removed or
/// renamed `default/` file changes it.  `None` when `default/` cannot be read.
#[cfg(feature = "mmap")]
fn stdlib_key_hex(default_dir: &str) -> Option<String> {
    let srcs = crate::cache::collect_stdlib_sources(default_dir);
    if srcs.is_empty() {
        return None;
    }
    Some(hex32(&crate::cache::stdlib_cache_key(&srcs)))
}

#[cfg(feature = "mmap")]
fn hex32(key: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in key {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Parse-time state a warm load cannot re-derive (it skips parsing), read back
/// from the manifest on a valid hit.
#[cfg(feature = "mmap")]
struct ManifestState {
    /// The `#cwd` directive's resolved path-resolution mode.
    program_relative: bool,
    /// `[library] native` registrations: `(stem, pkg_dir)` per native cdylib
    /// the parse registered (#310 — re-resolved at warm load so a cached run
    /// dlopens the same libraries, with cold-equal freshness checks).
    native_lib_regs: Vec<(String, String)>,
    /// `[native] crate` registrations: `(crate, pkg_dir)` per native package —
    /// what `Data::native_packages` holds (the rlib the `--native` path links).
    /// The IR bundle does not serialize it, and `--native` codegen maps every
    /// `#native` symbol to its owning crate through it — so without replaying it a
    /// warm `--native` build P269s a reachable `#native` fn as "no implementation
    /// in any registered native crate" (the ssh-lib regression).
    native_crate_regs: Vec<(String, String)>,
    /// @PLN119 — `[library] placement` registrations: `(name, spelling, pkg_dir)` per library
    /// that asked to run OUT OF PROCESS.  `mark_exports` writes its marks into `Data`, so the
    /// bundle already carries them — but the WORKER that those marked functions dispatch to is
    /// started from this list, and a warm load never ran the parse that built it.  Without the
    /// replay the marks point at `compile.rs`'s "native function not loaded" stub, so a placed
    /// library works on its first run and panics on its second.
    placed_libs: Vec<(String, String, String)>,
    /// Diagnostics the COLD parse produced, replayed on a warm load so a cached run says
    /// exactly what an uncached one says.  Without this the parser does not run, so nothing
    /// warns — the same program reports differently on its second run, and a library's CI
    /// (`LOFT_DENY_WARNINGS`) turns on whether anyone had run the build before.
    diagnostics: Vec<crate::diagnostics::DiagEntry>,
    /// Def-table index where USER definitions start (the stdlib def count when
    /// the user-file parse began).  A warm load restores stdlib + user defs in
    /// one table, so without this boundary the no-`main` test-fn fallback sees
    /// an empty user range and silently runs nothing (#358).
    user_def_start: Option<u32>,
    /// #444 — `[wasm.bridge]` state a `use`d library's manifest contributes:
    /// the `routes` map (`loft_sym → (crate, bridge_fn)`), the bridge crate
    /// packages, and the host-JS preamble files.  Manifest-derived parse state
    /// the IR bundle does not serialize, so — like `native_lib_regs` — a warm
    /// load must replay it or `--html` codegen sees an empty route table and
    /// emits a host-import `extern` for an already-routed `#native`, colliding
    /// (`E0428`) with the library's public wrapper of the same name.
    wasm_bridge_routes: Vec<(String, String, String)>,
    wasm_bridge_packages: Vec<(String, String)>,
    wasm_bridge_host_js: Vec<String>,
}

#[cfg(feature = "mmap")]
impl ManifestState {
    /// Whether the parse registered nothing beyond loft sources — no `[native] crate`,
    /// `[library] native`, placement or `[wasm.bridge]` entry.  What the source-keyed
    /// native fast path (@PLN166 B3) requires: each of those is something a warm load
    /// RE-RESOLVES (a cdylib's freshness, a worker to start), and a path that runs no
    /// parse-time code cannot.
    fn registers_only_loft_sources(&self) -> bool {
        self.native_lib_regs.is_empty()
            && self.native_crate_regs.is_empty()
            && self.placed_libs.is_empty()
            && self.wasm_bridge_routes.is_empty()
            && self.wasm_bridge_packages.is_empty()
            && self.wasm_bridge_host_js.is_empty()
    }
}

/// On a valid match, returns the parse-time [`ManifestState`] persisted in the
/// manifest.  `None` on a miss: absent manifest, stale build signature, or any
/// source drifted since the bundle was written.
#[cfg(feature = "mmap")]
fn manifest_state(manifest: &std::path::Path, stdlib_key: &str) -> Option<ManifestState> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let mut lines = text.lines();
    // @PLN11 G2/M6 — the first line pins THIS build's signature.  A binary
    // upgrade (new store layout / codegen / version / features) changes it, so a
    // stale bundle is a clean cache miss (reparse), never a wrong-format load
    // (`Store::is_store_file`'s fixed magic can't catch a layout change).
    match lines.next().and_then(|l| l.strip_prefix("sig ")) {
        Some(sig) if sig == crate::cache::build_signature() => {}
        _ => return None,
    }
    // `stdk <key>`: the stdlib this bundle was parsed against (`stdlib_key_hex`).  Required —
    // a bundle that cannot name its stdlib could be one built over a library that has since
    // changed, and serving it answers with the old library in silence.
    match lines.next().and_then(|l| l.strip_prefix("stdk ")) {
        Some(k) if k == stdlib_key => {}
        _ => return None,
    }
    // @PLN11 — optional `prel <0|1>` header: the parse-time `program_relative`
    // flag (the `#cwd` directive's resolved effect).  Present in manifests this
    // build wrote; absent only in pre-fix manifests, which the build-signature
    // bump already invalidates — so the `true` default is never actually read.
    let mut program_relative = true;
    let mut next = lines.next();
    if let Some(rest) = next.and_then(|l| l.strip_prefix("prel ")) {
        program_relative = rest != "0";
        next = lines.next();
    }
    // Optional `udef <n>` header (#358): where user defs start.  Like `prel`,
    // absent only in pre-fix manifests, which the build-signature bump already
    // invalidates — so the `None` fallback is never actually read.
    let mut user_def_start = None;
    if let Some(rest) = next.and_then(|l| l.strip_prefix("udef ")) {
        user_def_start = rest.parse::<u32>().ok();
        next = lines.next();
    }
    // Optional `nlib <stem> <pkg_dir>` headers (#310): one per `[library]
    // native` registration the parse performed.  A stem is a Rust crate stem
    // (no spaces), so the remainder after the second space is the package dir
    // verbatim — directories may contain spaces.
    let mut native_lib_regs = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("nlib ")) {
        let (stem, pkg_dir) = rest.split_once(' ')?;
        native_lib_regs.push((stem.to_string(), pkg_dir.to_string()));
        next = lines.next();
    }
    // Optional `ncrate <crate> <pkg_dir>` headers: one per `[native] crate`
    // registration (`Data::native_packages`) — replayed so a warm `--native`
    // build repopulates the native-symbol→crate map instead of P269-ing. A crate
    // name has no spaces, so the remainder after the first space is the dir.
    let mut native_crate_regs = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("ncrate ")) {
        let (krate, pkg_dir) = rest.split_once(' ')?;
        native_crate_regs.push((krate.to_string(), pkg_dir.to_string()));
        next = lines.next();
    }
    // Optional `plib <name> <spelling> <pkg_dir>` headers: one per out-of-process placed
    // library.  A package name and a placement spelling each have no spaces, so `splitn(3)`
    // leaves the package dir verbatim — directories may contain them.
    let mut placed_libs = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("plib ")) {
        let mut it = rest.splitn(3, ' ');
        let name = it.next()?.to_string();
        let spelling = it.next()?.to_string();
        let pkg_dir = it.next()?.to_string();
        placed_libs.push((name, spelling, pkg_dir));
        next = lines.next();
    }
    // Optional `diag <encoded>` headers: the diagnostics the cold parse emitted, one per
    // line (see `DiagEntry::encode_for_cache`).  A line that will not decode fails the whole
    // manifest — a bundle that cannot reproduce what the parse SAID must not be served,
    // because the failure mode is silence and silence reads as "no problem".
    let mut diagnostics = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("diag ")) {
        diagnostics.push(crate::diagnostics::DiagEntry::decode_from_cache(rest)?);
        next = lines.next();
    }
    // #444 — optional `wbroute <loft_sym> <crate> <bridge_fn>` headers: the
    // `[wasm.bridge].routes` map.  The three tokens are a `#native` symbol, a
    // crate name, and a bridge fn — none contains a space — so `splitn(3)` is
    // exact.  `wbpkg`/`wbhostjs` follow (their tail is a path that MAY contain
    // spaces, so they split only on the first / no space).
    let mut wasm_bridge_routes = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("wbroute ")) {
        let mut it = rest.splitn(3, ' ');
        let sym = it.next()?.to_string();
        let krate = it.next()?.to_string();
        let bridge_fn = it.next()?.to_string();
        wasm_bridge_routes.push((sym, krate, bridge_fn));
        next = lines.next();
    }
    let mut wasm_bridge_packages = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("wbpkg ")) {
        let (krate, pkg_dir) = rest.split_once(' ')?;
        wasm_bridge_packages.push((krate.to_string(), pkg_dir.to_string()));
        next = lines.next();
    }
    let mut wasm_bridge_host_js = Vec::new();
    while let Some(rest) = next.and_then(|l| l.strip_prefix("wbhostjs ")) {
        wasm_bridge_host_js.push(rest.to_string());
        next = lines.next();
    }
    // Remaining lines: `<hexhash> <path>` for every parsed source.
    let mut any = false;
    let mut cur = next;
    while let Some(line) = cur {
        let (hexhash, path) = line.split_once(' ')?;
        match crate::cache::file_hash(path) {
            Some(c) if hex32(&c) == hexhash => any = true,
            _ => return None,
        }
        cur = lines.next();
    }
    // An empty source list is not a valid hit.
    any.then_some(ManifestState {
        program_relative,
        native_lib_regs,
        native_crate_regs,
        placed_libs,
        diagnostics,
        user_def_start,
        wasm_bridge_routes,
        wasm_bridge_packages,
        wasm_bridge_host_js,
    })
}

/// Whole-program warm load: if a valid bundle exists for `script_abspath` and
/// every source it was built from is unchanged, load the entire `Data` + type
/// schema into `p` and return `Some(user_def_start)` — the def-table index
/// where user definitions begin (the caller then skips **all** parsing and
/// uses the boundary for the no-`main` test-fn fallback, #358).  `None` is a
/// cache miss: fall back to a cold parse.
#[cfg(feature = "mmap")]
#[must_use]
pub fn warm_load_program(
    p: &mut Parser,
    script_abspath: &str,
    default_dir: &str,
    store_out: &mut Option<(crate::database::Stores, crate::keys::DbRef)>,
) -> Option<u32> {
    let (bundle, manifest) = crate::cache::program_cache_paths(script_abspath, &p.lib_dirs);
    let state = manifest_state(&manifest, &stdlib_key_hex(default_dir)?)?;
    // #310 — re-resolve the parse-time `[library] native` registrations the
    // warm load skips, BEFORE committing to the bundle: each cdylib gets the
    // same prebuilt-or-auto-build freshness check a cold parse runs (a loft
    // rebuild still triggers the package rebuild).  Any failure → treat the
    // whole thing as a cache miss and let the cold path report it.
    let mut native_libs = Vec::new();
    for (stem, pkg_dir) in &state.native_lib_regs {
        let path = crate::extensions::resolve_native_lib(pkg_dir, stem)?;
        if !native_libs.contains(&path) {
            native_libs.push(path);
        }
    }
    // @PLN11 Arc N / N1 — touch-on-use: a warm hit marks the bundle recently-used
    // so the idle-TTL GC keeps actively-run programs and ages out one-offs.
    crate::cache::touch_now(&bundle);
    let bundle_s = bundle.to_string_lossy();
    // @PLN11 G2/M6 — with LOFT_CODEGEN_STORE, do a *skeleton* load: mmap the
    // bundle, reconstruct only the def table (bodies stay in the store), and
    // hand the store back so codegen reads bodies straight from it — skipping
    // read_data's body rebuild.  Otherwise full read_data (the M2/M5 default).
    let loaded = if std::env::var_os("LOFT_CODEGEN_STORE").is_some() {
        match crate::ir_read::open_program_store(&bundle_s) {
            Ok((stores, root, data, schema)) => {
                p.database.install_schema(schema);
                p.data = data;
                *store_out = Some((stores, root));
                true
            }
            Err(_) => false,
        }
    } else {
        match crate::ir_read::open_bundle_into(&bundle_s, &mut p.database) {
            Ok(data) => {
                p.data = data;
                true
            }
            Err(_) => false,
        }
    };
    if !loaded {
        return None;
    }
    // Repopulate `native_packages` (the `[native] crate` regs) — the IR bundle
    // stores only the def table, so a warm load loses them and `--native` codegen
    // P269s a reachable `#native` fn. Re-push them, then re-derive
    // `native_symbol_crates` via the SAME backfill the cold path runs after its
    // manifest registration (map each `#native` def to the package dir that is the
    // longest prefix of its source file).
    for (krate, pkg_dir) in &state.native_crate_regs {
        if !p.data.native_packages.iter().any(|(c, _)| c == krate) {
            p.data
                .native_packages
                .push((krate.clone(), pkg_dir.clone()));
        }
    }
    p.backfill_native_symbol_crates();
    // @PLN11 — restore the parse-time `#cwd` path-resolution mode the warm
    // load skipped.  Without it a cached `#cwd` program resolves relative
    // paths program-relative instead of cwd-relative, silently reading the
    // wrong base (e.g. the indexer scanning nothing).  `source_dir` needs no
    // such restore — main.rs sets it every run from the script path.
    p.database.program_relative = state.program_relative;
    // #310 — restore the native-cdylib registrations: without these,
    // `extensions::load_all` gets an empty list on warm runs and every
    // `#native` call hits the "native function not loaded" stub.
    p.pending_native_libs = native_libs;
    // @PLN119 — restore the out-of-process placement registrations.  `main` starts a worker
    // for each entry here and points the marked functions at it; the marks themselves came
    // back with the bundle (`mark_exports` writes them into `Data`), so without this the
    // marked calls resolve to the "native function not loaded" stub instead.  A spelling this
    // build cannot parse fails the whole warm load rather than dropping the library: falling
    // back to in-process would run the program correctly and isolate nothing, which is the one
    // outcome `Placement::parse` refuses for exactly this reason.
    for (name, spelling, pkg_dir) in &state.placed_libs {
        let placement = crate::lib_placement::Placement::parse(spelling).ok()?;
        p.pending_placed_libs
            .push((name.clone(), pkg_dir.clone(), placement));
    }
    // Replay what the cold parse said.  The parser did not run, so these are the only
    // diagnostics this run will have; `main` renders them through the same path a cold run
    // uses, so `LOFT_ERRORS`, colour and the warnings-off filter all still apply.
    for e in state.diagnostics {
        p.diagnostics.restore_from_cache(e);
    }
    p.native_lib_regs = state.native_lib_regs;
    // #444 — restore the `[wasm.bridge]` state the IR bundle does not carry.
    // `--html` codegen keys the host-import-extern skip AND the routed-call on
    // `wasm_bridge_routes`; an empty table makes those two decisions disagree
    // and collide (E0428).  `packages` drives the bridge-crate link and
    // `host_js` the HTML preamble — all three are parse-time-only, so the warm
    // path replays them here exactly as the cold parse populated them.
    for (sym, krate, bridge_fn) in state.wasm_bridge_routes {
        p.data.wasm_bridge_routes.insert(sym, (krate, bridge_fn));
    }
    p.data.wasm_bridge_packages = state.wasm_bridge_packages;
    p.data.wasm_bridge_host_js_files = state.wasm_bridge_host_js;
    Some(state.user_def_start.unwrap_or_else(|| p.data.definitions()))
}

/// Cold path: after the full parse, write the whole-program bundle + its drift
/// manifest for `script_abspath` (every deduped parsed source + its content
/// hash).  The bundle is published first, then the manifest atomically — so a
/// manifest is only ever present alongside a complete bundle.
#[cfg(feature = "mmap")]
pub fn save_program(
    p: &Parser,
    script_abspath: &str,
    default_dir: &str,
    user_def_start: u32,
    placed_libs: &[(String, String, crate::lib_placement::Placement)],
) {
    use std::fmt::Write as _;
    let Some(stdk) = stdlib_key_hex(default_dir) else {
        return; // no readable stdlib → a bundle that could not be tied to one
    };
    let (bundle, manifest) = crate::cache::program_cache_paths(script_abspath, &p.lib_dirs);

    let mut paths: Vec<&String> = p.parsed_sources.iter().collect();
    paths.sort_unstable();
    paths.dedup();
    let mut lines = String::new();
    // @PLN11 G2/M6 — pin the build signature first so a binary upgrade
    // invalidates this bundle (see `manifest_matches`).
    let _ = writeln!(lines, "sig {}", crate::cache::build_signature());
    let _ = writeln!(lines, "stdk {stdk}");
    // @PLN11 — persist the parse-time path-resolution mode (the `#cwd` directive's
    // resolved effect) so a warm load (which skips parsing) can restore it.
    let _ = writeln!(lines, "prel {}", u8::from(p.database.program_relative));
    // #358 — persist where user defs start so a warm load (which skips parsing)
    // can run the no-`main` test-fn fallback over exactly the user functions.
    let _ = writeln!(lines, "udef {user_def_start}");
    // #310 — persist each `[library] native` registration so a warm load can
    // re-resolve (and freshness-check) the cdylibs the parse registered.
    for (stem, pkg_dir) in &p.native_lib_regs {
        let _ = writeln!(lines, "nlib {stem} {pkg_dir}");
    }
    // Persist each `[native] crate` registration (`Data::native_packages`) so a
    // warm `--native` build repopulates the native-symbol→crate map — without it
    // the IR bundle (def table only) leaves it empty and codegen P269s a reachable
    // `#native` fn as "no implementation in any registered native crate".
    for (krate, pkg_dir) in &p.data.native_packages {
        let _ = writeln!(lines, "ncrate {krate} {pkg_dir}");
    }
    // @PLN119 — persist the out-of-process placement registrations.  Taken as an ARGUMENT
    // rather than read off the parser, because `main` has already `mem::take`n them by the
    // time this runs: the list drives both the native-candidate exclusion and the worker
    // install, so it is consumed before the bundle is written.
    for (name, pkg_dir, placement) in placed_libs {
        let _ = writeln!(lines, "plib {name} {} {pkg_dir}", placement.spelling());
    }
    // The diagnostics this parse produced, so a warm load can say what the cold run said.
    // Order is preserved: the renderer's warning-cascade dedup and the caller's
    // errors-only filter both depend on it.
    for e in p.diagnostics.entries() {
        let _ = writeln!(lines, "diag {}", e.encode_for_cache());
    }
    // #444 — persist the `[wasm.bridge]` state so a warm load reconstructs the
    // route table `--html` codegen reads (the IR bundle stores only the def
    // table).  Routes are sorted for a byte-stable manifest; the read side
    // rebuilds a `HashMap` so order is immaterial there.
    let mut routes: Vec<(&String, &(String, String))> = p.data.wasm_bridge_routes.iter().collect();
    routes.sort_by(|a, b| a.0.cmp(b.0));
    for (sym, (krate, bridge_fn)) in routes {
        let _ = writeln!(lines, "wbroute {sym} {krate} {bridge_fn}");
    }
    for (krate, pkg_dir) in &p.data.wasm_bridge_packages {
        let _ = writeln!(lines, "wbpkg {krate} {pkg_dir}");
    }
    for host_js in &p.data.wasm_bridge_host_js_files {
        let _ = writeln!(lines, "wbhostjs {host_js}");
    }
    for path in &paths {
        let Some(h) = crate::cache::file_hash(path) else {
            return; // an unreadable source → don't cache (would never validate)
        };
        let _ = writeln!(lines, "{} {path}", hex32(&h));
    }
    if paths.is_empty() {
        return; // no sources → an unvalidatable manifest; skip caching
    }

    if let Some(parent) = bundle.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if crate::ir_store::save_bundle(&p.data, &p.database.types, &bundle.to_string_lossy()).is_err()
    {
        return;
    }
    // Manifest last + atomically, through a tmp name of THIS PROCESS's own — the same shape
    // `ir_store::save_bundle` uses for the bundle beside it, and for the same reason.  A fixed
    // `.manifest.tmp` is shared by every loft writing the same program, so two of them
    // interleave their bytes into one file and both rename it: the reader then validates a
    // manifest that is a MIX of two writes.  Its `diag` lines are the visible half — a warm
    // load replays fewer diagnostics than the cold run produced, so one side of a
    // parity comparison reports a warning the other does not (loft#1129).  Concurrent loft
    // processes over one bundle are not a test artefact: two builds at once is an ordinary
    // thing for a user to do.
    let tmp = manifest.with_extension(format!("manifest.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, lines.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &manifest);
    }
    let _ = std::fs::remove_file(&tmp);
    // @PLN11 G2 / track 1 — with the cache default-on, bound the directory size
    // by evicting the oldest bundles after each cold save.
    crate::cache::prune_program_cache();
}

/// @PLN166 B3 — what a native run needs to exec a cached binary WITHOUT parsing: the
/// binary, and the two parse-time facts the driver would otherwise take from the parse.
pub struct NativeHit {
    /// The cached binary the sidecar names; the caller still runs the P254 safety check.
    pub binary: std::path::PathBuf,
    /// The `#cwd` directive's resolved effect (the binary is run with cwd = source dir
    /// under it, exactly as after a parse).
    pub program_relative: bool,
    /// What the cold parse said, replayed so this run says it too.
    pub diagnostics: Vec<crate::diagnostics::DiagEntry>,
}

/// @PLN166 B3 — the source-keyed native fast path.  `Some` when the binary built from
/// THIS program, under THIS build and `fingerprint`, is provably the one a parse + codegen +
/// rustc would produce again: the program's drift manifest validates (every parsed source's
/// content hash and the stdlib key — the same oracle the interpreter trusts to skip parsing),
/// the sidecar's build signature and fingerprint match, and the manifest registers nothing
/// a warm load would have to re-resolve.  `None` is a miss; the caller then takes the path
/// that parses, generates and checks the binary cache by the generated Rust as before.
///
/// **Only a pure-loft program is served** — no `[native] crate`, `[library] native`,
/// placement or `[wasm.bridge]` registration.  Those are the inputs a warm load re-resolves
/// (a cdylib's freshness, a worker to start), and the fast path runs no code that could;
/// refusing them keeps every such program on the path that computes, at the cost of the
/// ~10 ms this path saves.  `fingerprint` is the caller's hash over every remaining codegen
/// and rustc input (flags, rlib identity, rustflags); the env policy is the caller's too.
#[cfg(feature = "mmap")]
#[must_use]
pub fn native_fast_path(
    script_abspath: &str,
    lib_dirs: &[String],
    default_dir: &str,
    fingerprint: u64,
) -> Option<NativeHit> {
    let (bundle, manifest) = crate::cache::program_cache_paths(script_abspath, lib_dirs);
    let sidecar = crate::cache::native_sidecar_path(&manifest);
    // The sidecar first: three cheap lines before the manifest's hash walk.
    let text = std::fs::read_to_string(&sidecar).ok()?;
    let mut lines = text.lines();
    match lines.next().and_then(|l| l.strip_prefix("sig ")) {
        Some(sig) if sig == crate::cache::build_signature() => {}
        _ => return None,
    }
    match lines.next().and_then(|l| l.strip_prefix("fp ")) {
        Some(fp) if fp == format!("{fingerprint:016x}") => {}
        _ => return None,
    }
    let binary = std::path::PathBuf::from(lines.next()?.strip_prefix("bin ")?);
    // A line this build does not know is a sidecar this build did not write.
    if lines.next().is_some() {
        return None;
    }
    if !binary.is_file() {
        return None;
    }
    let state = manifest_state(&manifest, &stdlib_key_hex(default_dir)?)?;
    if !state.registers_only_loft_sources() {
        return None;
    }
    // A hit is a use: keep the bundle the sidecar depends on out of the idle-TTL sweep.
    crate::cache::touch_now(&bundle);
    Some(NativeHit {
        binary,
        program_relative: state.program_relative,
        diagnostics: state.diagnostics,
    })
}

/// @PLN166 B3 — record that `binary` is the native binary of the program whose manifest
/// is current, under `fingerprint`.  Written only beside an EXISTING manifest, because the
/// manifest is what validates the sources on the next run; atomically, through a tmp name
/// of this process's own, for the same reason the manifest is (loft#1129).
#[cfg(feature = "mmap")]
pub fn save_native_sidecar(
    script_abspath: &str,
    lib_dirs: &[String],
    fingerprint: u64,
    binary: &std::path::Path,
) {
    let (_, manifest) = crate::cache::program_cache_paths(script_abspath, lib_dirs);
    if !manifest.is_file() {
        return;
    }
    let Some(binary) = binary.to_str() else {
        return;
    };
    let sidecar = crate::cache::native_sidecar_path(&manifest);
    let body = format!(
        "sig {}\nfp {fingerprint:016x}\nbin {binary}\n",
        crate::cache::build_signature()
    );
    let tmp = sidecar.with_extension(format!("native.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, body.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &sidecar);
    }
    let _ = std::fs::remove_file(&tmp);
}

#[cfg(not(feature = "mmap"))]
#[must_use]
pub fn native_fast_path(
    _script_abspath: &str,
    _lib_dirs: &[String],
    _default_dir: &str,
    _fingerprint: u64,
) -> Option<NativeHit> {
    None
}
#[cfg(not(feature = "mmap"))]
pub fn save_native_sidecar(
    _script_abspath: &str,
    _lib_dirs: &[String],
    _fingerprint: u64,
    _binary: &std::path::Path,
) {
}

/// Non-`mmap` builds: the whole-program cache is unavailable.
#[cfg(not(feature = "mmap"))]
#[must_use]
pub fn warm_load_program(
    _p: &mut Parser,
    _script_abspath: &str,
    _default_dir: &str,
    _store_out: &mut Option<(crate::database::Stores, crate::keys::DbRef)>,
) -> Option<u32> {
    None
}
#[cfg(not(feature = "mmap"))]
pub fn save_program(
    _p: &Parser,
    _script_abspath: &str,
    _default_dir: &str,
    _user_def_start: u32,
    _placed_libs: &[(String, String, crate::lib_placement::Placement)],
) {
}

#[cfg(all(test, feature = "mmap"))]
mod ncrate_manifest_tests {
    use super::*;

    /// The cache manifest must replay an `ncrate <crate> <pkg_dir>` header so a warm `--native`
    /// build repopulates `native_packages` (the ssh-lib P269 regression). Exercises the read
    /// side of that round-trip beside the existing `nlib` header.
    #[test]
    fn manifest_state_parses_ncrate_native_packages() {
        let dir = std::env::temp_dir().join(format!("loft_ncrate_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("s.loft");
        std::fs::write(&src, "fn main() {}\n").unwrap();
        let src_str = src.to_string_lossy().to_string();
        let hash = crate::cache::file_hash(&src_str).expect("hash source");
        let manifest = dir.join("m.manifest");
        let content = format!(
            "sig {}\nstdk k\nnlib loft_foo /pkgs/foo\nncrate loft-foo /pkgs/foo\n{} {}\n",
            crate::cache::build_signature(),
            hex32(&hash),
            src_str,
        );
        std::fs::write(&manifest, &content).unwrap();

        let state = manifest_state(&manifest, "k").expect("valid manifest hit");
        assert_eq!(
            state.native_crate_regs,
            vec![("loft-foo".to_string(), "/pkgs/foo".to_string())],
            "the ncrate header must round-trip into native_crate_regs"
        );
        assert_eq!(
            state.native_lib_regs,
            vec![("loft_foo".to_string(), "/pkgs/foo".to_string())],
            "the sibling nlib header still parses beside ncrate"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// @PLN166 B3 — the source-keyed native fast path serves only a program whose manifest
    /// registers nothing beyond loft sources.  One registration of any kind — here an
    /// `ncrate`, the case that P269'd a warm `--native` build once — is enough to refuse.
    #[test]
    fn the_native_fast_path_refuses_a_manifest_with_a_registration() {
        let dir = std::env::temp_dir().join(format!("loft_nsk_pure_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("s.loft");
        std::fs::write(&src, "fn main() {}\n").unwrap();
        let src_str = src.to_string_lossy().to_string();
        let source_line = format!(
            "{} {}",
            hex32(&crate::cache::file_hash(&src_str).unwrap()),
            src_str
        );
        let manifest = dir.join("m.manifest");
        let sig = crate::cache::build_signature();
        for (header, pure) in [
            ("", true),
            ("ncrate loft-foo /pkgs/foo\n", false),
            ("nlib loft_foo /pkgs/foo\n", false),
            ("plib foo worker /pkgs/foo\n", false),
            ("wbroute n_x foo bridge_x\n", false),
            ("wbpkg foo /pkgs/foo\n", false),
            ("wbhostjs /pkgs/foo/host.js\n", false),
        ] {
            std::fs::write(
                &manifest,
                format!("sig {sig}\nstdk k\n{header}{source_line}\n"),
            )
            .unwrap();
            let state = manifest_state(&manifest, "k")
                .unwrap_or_else(|| panic!("the manifest with {header:?} must validate"));
            assert_eq!(
                state.registers_only_loft_sources(),
                pure,
                "a manifest carrying {header:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
