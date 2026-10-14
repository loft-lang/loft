// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: finding or building a package's native library

//! Finding a `use`d package's native library — a prebuilt cdylib for this host, else a
//! source build — and the facts a build needs (the target root, the remap flags, the
//! missing-system-library diagnostic).  The parser asks it when it resolves a `use`, so it
//! sits in the core layer; loading the library and calling into it stay in `extensions`
//! (@PLN191).

use crate::platform::process::{Program, Spawn};

pub(crate) fn local_remap_flags() -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // The toolchain sysroot first: it lives inside the rustup home and carries
    // the toolchain's own directory NAME, so a build on `stable` and one pinned
    // to an exact version would otherwise still differ.
    if let Ok(o) = Spawn::new(Program::os(
        std::env::var("RUSTC").as_deref().unwrap_or("rustc"),
    ))
    .arg("--print")
    .arg("sysroot")
    .run(b"")
        && o.status.success()
    {
        let root = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !root.is_empty() {
            let _ = write!(out, "--remap-path-prefix={root}=/rustc ");
        }
    }
    let cargo_home = std::env::var("CARGO_HOME")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.cargo")));
    if let Some(c) = cargo_home {
        let _ = write!(out, "--remap-path-prefix={c}=/cargo");
    }
    out
}

/// Auto-build a package's native crate if the shared library OR
/// rlib is missing.
///
/// Both artifacts are produced by the same `cargo build` invocation
/// (every `lib/*/native/Cargo.toml` declares
/// `crate-type = ["cdylib", "rlib"]`).  The cdylib is loaded by the
/// runtime extension loader; the rlib is consumed by `--native`
/// codegen via `--extern <crate>=<path>` (see
/// `src/main.rs::add_native_extern_flags`).
///
/// Pre-2026-05-12 this function checked ONLY the cdylib before
/// returning early, which let a parallel test see "cdylib exists"
/// → skip the build → then fail later when `add_native_extern_flags`
/// looked for the rlib that hadn't been written yet.  On Windows
/// (where file-system flush ordering between cdylib and rlib differs
/// from ext4/APFS, and file-locks held by another concurrent cargo
/// invocation block reads even when the file is on disk) this race
/// surfaced as `error[E0463]: can't find crate for <name>` in
/// `tests/codegen_emitter.rs::p244_text_native_wrapper_compiles_under_native`
/// and any other test that ran in parallel with a fresh build.
///
/// Fix: also require the rlib to exist before returning early.  If
/// either artifact is missing, run cargo (which is itself
/// fcntl-locked per target dir, so concurrent invocations serialize
/// and only one actually rebuilds).
/// @PLAN12 Phase 6b — compute the target dir for a package's
/// native cdylib/rlib build.
///
/// When `pkg_dir` is under `~/.loft/registry/` (i.e., a
/// `loft install`-extracted chunk), returns the redirected
/// `~/.loft/build-cache/<pkg>-<ver>/` path; otherwise returns
/// `<pkg_dir>/native/target` (the in-tree default).
///
/// Used by [`auto_build_native`] (to set `CARGO_TARGET_DIR` and
/// to read the freshly-built artifact) AND by
/// `native_utils::add_native_extern_flags` (to find the rlib at
/// link time).  Single source of truth so the cdylib/rlib are
/// always at the same root.
#[must_use]
pub fn native_target_root(pkg_dir: &std::path::Path) -> std::path::PathBuf {
    // Without the registry feature there is no `~/.loft/registry/` to
    // redirect away from — every install is in-tree, so the package
    // dir's own `native/target/` is the target root.
    #[cfg(not(feature = "registry"))]
    {
        pkg_dir.join("native").join("target")
    }
    #[cfg(feature = "registry")]
    {
        let registry_cache = crate::registry_index::cache_dir();
        let registry_cache_canon = crate::file_access::try_plain_canonical(&registry_cache);
        let pkg_canon = crate::file_access::try_plain_canonical(std::path::Path::new(pkg_dir));
        let use_redirected = match (&registry_cache_canon, &pkg_canon) {
            (Some(rc), Some(pc)) => pc.starts_with(rc),
            _ => false,
        };
        if use_redirected {
            let stem_dir =
                crate::file_access::file_name(pkg_dir).unwrap_or_else(|| "native-pkg".to_string());
            registry_cache
                .parent()
                .map_or_else(
                    || std::path::PathBuf::from("."),
                    std::path::Path::to_path_buf,
                )
                .join("build-cache")
                .join(stem_dir)
        } else {
            pkg_dir.join("native").join("target")
        }
    }
}

/// Resolve a `[library] native = "<stem>"` registration to a loadable cdylib
/// path: a pre-built `<pkg_dir>/native/<libname>` wins, else build (or reuse a
/// fingerprint-fresh build of) the package's native crate via
/// [`auto_build_native`].  The ONE home for this resolution — the parser's two
/// manifest paths and the warm startup-cache load (#310) all derive from it,
/// so a cached run re-checks cdylib freshness exactly like a cold parse.
/// Set for a `loft --html` run: the program is built for the browser, which never loads a
/// package's HOST native library — the page answers the package's imports itself.  So the
/// parse registers no host cdylib and the driver builds no default-native one, where both
/// used to run a full cargo build of code the build never links (`html_gl_imports` spent
/// minutes of a 327 s run there on a cold runner).  The program cache keys on `--html`
/// (`startup_cache::native_lib_context`), so a browser run's bundle is never warm-loaded by
/// a run that needs those libraries.
pub(crate) static BROWSER_TARGET: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Mark this run as a browser build (`loft --html`), before anything resolves a package.
pub fn set_browser_target() {
    BROWSER_TARGET.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Is this run a browser build?  Then no host native library is built or registered.
#[must_use]
pub fn browser_target() -> bool {
    BROWSER_TARGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// The host native library a `use`d package registers — none for a browser build, which
/// never loads one ([`browser_target`]); otherwise [`resolve_native_lib`].
#[must_use]
pub fn host_native_lib(pkg_dir: &str, stem: &str) -> Option<String> {
    if browser_target() {
        return None;
    }
    resolve_native_lib(pkg_dir, stem)
}

pub fn resolve_native_lib(pkg_dir: &str, stem: &str) -> Option<String> {
    // @PLN21 Phase 3 — a missing DECLARED runtime system lib is terminal:
    // neither a prebuilt nor a source build can load it (both link the same lib,
    // decision C3).  Check FIRST and emit an actionable hint, rather than loading
    // a doomed prebuilt or spending ~90s on a build that cannot load.
    if let Some(lib) = first_missing_runtime_lib(pkg_dir) {
        eprintln!("{}", runtime_lib_missing_diagnostic(stem, &lib));
        return None;
    }
    let filename = platform_lib_name(stem);
    // @PLN21 Phase 1 — a precompiled cdylib shipped for THIS host triple wins
    // over a source build (the "no rustc to use a library" path).  A cdylib
    // links loft-ffi (the C-ABI), not libloft, so it is valid for any loft on
    // the same loft-ffi version — gated on the `.loft-build-fp` sidecar matching
    // `loft_ffi_fingerprint()`, so a binary built against a different loft-ffi
    // is skipped (never mis-loaded), falling through to a source build.
    let triple_dir = format!("{pkg_dir}/prebuilt/{}", crate::cache::host_triple());
    let triple_lib = format!("{triple_dir}/{filename}");
    if crate::file_access::exists(&triple_lib)
        && crate::cache::native_artifact_fingerprint_matches(
            std::path::Path::new(&triple_dir),
            crate::cache::loft_ffi_fingerprint(),
        )
    {
        crate::platform::timing_record("prebuilt", stem, true, None);
        return Some(triple_lib);
    }
    // Legacy platform-agnostic prebuilt (existence-only; kept for back-compat).
    let prebuilt = format!("{pkg_dir}/native/{filename}");
    if crate::file_access::exists(&prebuilt) {
        return Some(prebuilt);
    }
    auto_build_native(pkg_dir, stem)
}

/// The host OS as a display name, for runtime-lib diagnostics + platform-scoping.
/// Not feature-gated: `runtime_lib_missing_diagnostic` (and thus its call site in
/// the unconditionally-compiled `resolve_native_lib`) needs it even when
/// `native-extensions` is off (e.g. the WASM build).
pub(crate) fn host_os_name() -> &'static str {
    crate::platform::host_os_name()
}

#[cfg(not(feature = "native-extensions"))]
pub(crate) fn first_missing_runtime_lib(_pkg_dir: &str) -> Option<String> {
    None
}

/// Diagnostic for a host-applicable `[native] runtime-libs` entry that could not
/// be `dlopen`'d — the library is genuinely not installed.  Foreign-OS names are
/// skipped before this point (see [`first_missing_runtime_lib`]), so they never
/// reach here and never produce misleading "install it" advice.  Not
/// feature-gated: its call site in `resolve_native_lib` compiles unconditionally
/// (the `first_missing_runtime_lib` if-branch is dead but still type-checked when
/// `native-extensions` is off), so the helper must exist in every build.
pub(crate) fn runtime_lib_missing_diagnostic(stem: &str, lib: &str) -> String {
    format!(
        "loft: native library '{stem}' needs the system library '{lib}', which is not installed \
         on {host} ({triple}) — install it with your OS package manager (building from source \
         would link the same library and fail identically).",
        host = host_os_name(),
        triple = crate::cache::host_triple(),
    )
}

/// The link flags that make a built cdylib RELOCATABLE — empty on every platform but macOS
/// (`platform::relocatable_dylib_flags`, which says why).
pub(crate) fn relocatable_dylib_flags(lib_name: &str) -> String {
    crate::platform::relocatable_dylib_flags(lib_name)
}

#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn auto_build_native(pkg_dir: &str, stem: &str) -> Option<String> {
    use std::path::PathBuf;
    // P244-windows fix #2 (2026-05-12): use PathBuf::join, not
    // `format!("{pkg_dir}/...")`.  When `pkg_dir` arrives as a
    // canonicalized Windows extended-length path (e.g.
    // `\\?\D:\a\loft\loft\lib\server`), concatenating with `/native/...`
    // produces a malformed path: the `\\?\` verbatim prefix bypasses
    // Rust's path normalization, so the mixed-separator suffix
    // doesn't resolve and `Path::exists()` returns false even when
    // the file is on disk.  PathBuf::join handles each component
    // through proper Path semantics on every platform.
    let pkg = PathBuf::from(pkg_dir);
    let cargo_toml = pkg.join("native").join("Cargo.toml");
    if !crate::file_access::exists(&cargo_toml) {
        return None;
    }
    let lib_name = platform_lib_name(stem);
    let rlib_name = format!("lib{stem}.rlib");

    // @PLAN12 Phase 6b — target root via the shared helper (redirects
    // chunk-resident installs to ~/.loft/build-cache/<pkg>-<ver>/;
    // keeps in-tree target/ for the monorepo's lib/<pkg>/native/).
    let target_root = native_target_root(&pkg);
    let in_tree_target = pkg.join("native").join("target");
    let use_redirected_target = target_root != in_tree_target;

    // Existing-cache check: look in the redirected target first
    // (where future builds land), then the legacy in-tree target/
    // (so existing builds from older loft binaries are still
    // reused).
    let mut search_roots: Vec<PathBuf> = Vec::new();
    search_roots.push(target_root.clone());
    if use_redirected_target {
        search_roots.push(in_tree_target.clone());
    }
    // Reuse a cached cdylib only when it was built against the same loft-ffi ABI, the
    // same RUSTFLAGS and the same loft version.  This crate is HAND-WRITTEN: it LINKS
    // loft-ffi (the C-ABI), never libloft.rlib (verified: zero loft undefined symbols),
    // and depends on no loft codegen — so the loft build is deliberately NOT in the key
    // (`native_artifact_cache_key`, @PLN159 phase C; the generated `loft_auto_*` cdylib
    // is the one a codegen change reaches, and it carries the rlib hash in its name).
    // Profile-independent, so debug/release/test in a job SHARE the cache; it flips on
    // a real loft-ffi change, a flag change (#274 — `-g` ↔ not, the shared-dep SVH fix)
    // or a version bump, and the crate's own sources are checked by mtime (loft#965).
    let fp = crate::cache::native_artifact_cache_key();
    let find_existing = || {
        for root in &search_roots {
            for profile in ["release", "debug"] {
                let dir = root.join(profile);
                let lib = dir.join(&lib_name);
                let rlib = dir.join(&rlib_name);
                if crate::file_access::exists(&lib)
                    && crate::file_access::exists(&rlib)
                    && crate::cache::native_artifact_fingerprint_matches(&dir, fp)
                    // loft#965 — and the crate's OWN Rust sources are not newer.  The
                    // fingerprint answers "same loft-ffi ABI, RUSTFLAGS and codegen
                    // version", which are all questions about the environment; it says
                    // nothing about the hand-written code being compiled.  So editing a
                    // `#native` package's `native/src/lib.rs` and running `loft test`
                    // loaded the PREVIOUS build and passed — a false GREEN, and one only
                    // a local author sees, because a clean CI checkout has no artifact
                    // to reuse.
                    && !native_crate_newer_than(pkg_dir, &lib)
                {
                    return Some(lib.to_string_lossy().to_string());
                }
            }
        }
        None
    };
    if let Some(p) = find_existing() {
        crate::platform::timing_record("cdylib", stem, true, None);
        return Some(p);
    }

    // Visibility (building on loft2's loud-fallback commit): distinguish a COLD
    // miss (nothing cached) from a STALE REJECTION — the cdylib + rlib EXIST but
    // their stamped fingerprint != THIS build's fp, so they are rebuilt.  Across
    // CI runs that is the dominant cost: every loft source commit flips
    // `loft_build_fingerprint` (the libloft.rlib content hash), rejecting cdylibs
    // that actually only depend on the stable published loft-ffi ABI.  Log the
    // stamped-vs-current fp (so two runs reveal whether the rlib hash flipped on
    // an IDENTICAL commit — a second, non-reproducible-build instability — or
    // only across commits) and record a `cdylibstale` event the CI timing step
    // sums into wasted-rebuild seconds.  Pure diagnostics — no gate change yet.
    'scan: for root in &search_roots {
        for profile in ["release", "debug"] {
            let dir = root.join(profile);
            if crate::file_access::exists(dir.join(&lib_name))
                && crate::file_access::exists(dir.join(&rlib_name))
            {
                let stamped = crate::cache::native_artifact_stamped_fp(&dir)
                    .map_or_else(|| "none".to_string(), |s| s.to_string());
                eprintln!(
                    "loft: note — cdylib {stem} rebuilt: cached artifact rejected \
                     (stamped loft-ffi fp={stamped} != current fp={fp}) — loft-ffi's source \
                     changed since it was built. (Keyed on loft-ffi, not libloft.rlib, so a \
                     plain interpreter change no longer triggers this.)"
                );
                // Encode stamped|cur into the ledger name so the fp values
                // survive on a PASSING run (nextest hides the eprintln above);
                // the CI step strips the suffix for the per-package join.
                let tagged = format!("{stem}|stamped={stamped}|cur={fp}");
                crate::platform::timing_record("cdylibstale", &tagged, false, None);
                break 'scan;
            }
        }
    }

    // @P388: serialise on-demand native builds ACROSS PROCESSES.  Parallel test
    // binaries (nextest runs one process per test) and parallel end-user `loft`
    // invocations otherwise each run an unlocked `cargo build` that re-resolves
    // the dependency tree concurrently, racing cargo's shared registry index +
    // package cache → transient "required to be available in rlib format" errors
    // and non-deterministic version picks.  A single cross-process advisory file
    // lock serialises the builds.  (The CI pre-build already serialises CI; this
    // covers the latent end-user-parallelism half of @P388.)  The `--locked`
    // route is unavailable: the repo deliberately gitignores every `Cargo.lock`,
    // and these crates.io-sourced native deps would drift against the actively-
    // versioned loft-ffi crates.  Best-effort: if the lock can't be opened/taken
    // we proceed unserialised — no worse than before.  `File::lock` blocks until
    // acquired and releases when `_build_lock` drops at function exit (so a crash
    // mid-build can't strand the lock).
    let _build_lock = crate::file_access::open_with(
        std::env::temp_dir().join("loft-native-build.lock"),
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false),
    )
    .ok();
    if let Some(f) = &_build_lock {
        // Reveal cross-process contention on this ONE global lock (the prime
        // suspect for the CI 600s multiplayer hang): record `lockwait` BEFORE
        // blocking (so it lands in the ledger even if this process is killed
        // while still waiting), then `lockheld` with the wait duration once
        // acquired.  A `lockwait` with no matching `lockheld` for a package =
        // a process stuck behind another's long cold build.
        let lock_t = std::time::Instant::now();
        crate::platform::timing_record("lockwait", stem, false, None);
        // loft#1238 — and tell the TIMEOUT what we are waiting for.  The ledger already
        // distinguished "still waiting" (a `lockwait` with no `lockheld`) from "waited and
        // built", but only to a reader who has the ledger AND knows that convention; the kill
        // message said `phase=parse`, which is true and useless.  Now the process that dies here
        // says so in the line that reports its death.
        let waiting = format!("the global native-build lock (for {stem})");
        // loft#1238 — bounded by the run's remaining budget.  Waiting past the deadline cannot
        // succeed: the process is killed mid-queue, builds nothing, stamps nothing, and the next
        // attempt repeats it.  Failing here instead is the same outcome said out loud.
        if let Err(waited) = crate::timeout::lock_within_budget(f, &waiting) {
            crate::platform::timing_record("lockgaveup", stem, false, Some(waited.as_secs_f64()));
            if crate::timeout::first_giveup_report(stem) {
                eprintln!(
                    "loft: TRANSIENT — gave up waiting for {waiting} after {:.1}s.\n  \
                     Another process is building a native library and this run's timeout \
                     would expire before the queue cleared, so it stopped rather than be \
                     killed mid-wait.\n  Nothing here is broken or stale in the way the \
                     next message will suggest: `{stem}`'s cdylib simply had not been \
                     rebuilt yet. Build it once up front (`loft cache warm`), raise the \
                     timeout, or re-run once the other build has finished.",
                    waited.as_secs_f64()
                );
            }
            return None;
        }
        crate::platform::timing_record(
            "lockheld",
            stem,
            false,
            Some(lock_t.elapsed().as_secs_f64()),
        );
    }
    // Re-check under the lock: a process we waited on may have just produced the
    // artifact, in which case we must NOT rebuild.
    if let Some(p) = find_existing() {
        return Some(p);
    }

    // NO rustc-version guard here, deliberately: a package's native crate
    // depends on loft-ffi (the C-ABI contract) — never the SVH-locked loft
    // rlib — so ANY rustc builds it correctly (cargo itself rebuilds when
    // the resolved toolchain flips between invocation cwds).  The guard
    // belongs only to builds that link the rlib (`build_shared_cdylib`,
    // the driver's program-native path).

    // An artifact EXISTS here but its fingerprint names another loft build —
    // cargo cannot be trusted to rebuild it, and the post-build stamp would
    // launder it as fresh (see `cache::clear_stale_native_target`).  Only
    // the REDIRECTED root (`~/.loft/build-cache/…`) is cleared: it is
    // private to this resolution path (every consumer arrives through the
    // build lock above), whereas an in-tree `native/target` is shared with
    // direct-path consumers — the tests/lib fixture loaders read the `.so`
    // path without the lock, so a wipe there races them (suite-caught) —
    // and in-tree crates remain the documented dev workflow
    // (`make rebuild-native-cdylibs` / the stub-panic rebuild hint).
    if use_redirected_target {
        crate::cache::clear_stale_native_target(&target_root, &lib_name, &rlib_name, fp);
    }

    // Build.  When redirecting, pass `CARGO_TARGET_DIR` so cargo writes outside
    // the install dir.  Factored into a closure so a `--locked` failure can
    // retry without it (below) — the two invocations differ only in that flag.
    let make_cmd = |locked: bool| {
        let mut cmd = Spawn::new(Program::search("cargo"))
            .args(["build", "--release", "--manifest-path"])
            .arg(&cargo_toml)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        if locked {
            cmd = cmd.arg("--locked");
        }
        // #274 — build the package crate with the SAME RUSTFLAGS loft's own
        // rlibs used (captured at loft build time), so a shared transitive dep
        // like `libloading` gets a matching SVH.  Otherwise loft's `-g` copy and
        // the package's plain copy share a `StableCrateId` but differ in SVH and
        // rustc aborts at the generated program's link step.  Force it
        // (overriding any ambient value) and clear `CARGO_ENCODED_RUSTFLAGS` so
        // cargo doesn't see both forms at once.
        // The baked flags carry no `--remap-path-prefix` (build.rs strips them:
        // they name the RELEASE machine's paths).  Recompute them for THIS
        // machine so loft's rlibs and the package crate agree on `/cargo` and
        // `/rustc`, which is what #274's SVH match actually needs — and which
        // also keeps the consumer's cdylib free of their own home directory.
        let flags = format!(
            "{} {} {}",
            env!("LOFT_BUILD_RUSTFLAGS"),
            local_remap_flags(),
            relocatable_dylib_flags(&lib_name)
        );
        cmd = cmd
            .env("RUSTFLAGS", flags.trim())
            .env_remove("CARGO_ENCODED_RUSTFLAGS");
        // No post-link `strip` on macOS — see `relocatable_dylib_flags`.
        for (key, value) in crate::platform::cdylib_build_env() {
            cmd = cmd.env(key, value);
        }
        if use_redirected_target {
            if let Some(parent) = target_root.parent() {
                let _ = crate::file_access::create_dir_all(parent);
            }
            cmd = cmd.env("CARGO_TARGET_DIR", &target_root);
        }
        cmd
    };
    let built_path = target_root.join("release").join(&lib_name);
    let build_start = std::time::Instant::now();
    // @PLN21 Phase 6 — prefer `--locked` when the package SHIPS a
    // `native/Cargo.lock`: cargo then uses the pinned resolution (reproducible —
    // two machines produce the same cdylib bytes).  But a shipped lock can be
    // platform-INCOMPLETE: one generated on Linux lacks a crate's `cfg(windows)`
    // deps (e.g. `windows-targets`), so `cargo build --locked` on Windows must
    // update the lock and refuses ("cannot update the lock file … because
    // --locked was passed").  A FALLBACK source build must never hard-fail on
    // that (Goal F), so on a locked failure we retry WITHOUT --locked, warning
    // it is non-reproducible — reproducibility is best-effort here; the
    // submit-time gate is where a complete lock is enforced.
    let has_lock = crate::file_access::exists(pkg.join("native").join("Cargo.lock"));
    let mut status = make_cmd(has_lock).status();
    if has_lock && matches!(&status, Ok(s) if !s.success()) {
        eprintln!(
            "loft: '--locked' build of '{stem}' failed — its Cargo.lock can't be \
             satisfied as-is on this host (often a platform-specific dep missing \
             from the shipped lock); retrying without --locked (non-reproducible)."
        );
        status = make_cmd(false).status();
    }
    crate::platform::timing_record(
        "cdylib",
        stem,
        false,
        Some(build_start.elapsed().as_secs_f64()),
    );
    match status {
        Ok(s) if s.success() => {
            // @PLN11 Arc N / N0 — stamp the build fingerprint on ANY successful
            // cargo build: the rlib is produced even for rlib-only packages whose
            // cdylib `built_path` is absent, so a later loft change still
            // invalidates it (see `find_existing` / `add_native_extern_flags`).
            crate::cache::write_native_artifact_fingerprint(&target_root.join("release"), fp);
            crate::file_access::exists(&built_path)
                .then(|| built_path.to_string_lossy().to_string())
        }
        // @PLN21 Phase 3 — the build RAN but failed (cargo's error is on the
        // inherited stderr).  The usual cause is a missing build dependency, so
        // name the package's declared `build-deps` rather than leaving the user
        // to parse a raw rustc/linker/pkg-config error.
        Ok(_) => {
            eprintln!(
                "loft: building native library '{stem}' from source failed (cargo error above).{}",
                build_deps_hint(pkg_dir)
            );
            None
        }
        // cargo itself could not start — typically no Rust toolchain installed.
        Err(e) => {
            eprintln!(
                "loft: cannot build native library '{stem}': {e} — building a library with no \
                 prebuilt for this host needs a Rust toolchain (rustc + cargo)."
            );
            None
        }
    }
}

/// @PLN26 phase 3 — cross-build a `#native` package's crate to a wasm `target`
/// (`wasm32-wasip2` for `--native-wasm`, `wasm32-unknown-unknown` for `--html`) so the
/// wasm linker can consume its **rlib**, the way [`auto_build_native`] produces the host
/// cdylib.  The rlib lands at the IN-TREE `<pkg>/native/target/<target>/release/lib<stem>.rlib`
/// — exactly the path `native_utils::add_native_extern_flags` reads for a wasm target.
///
/// Returns whether that rlib is present afterwards: reused when it is already stamped with
/// the current loft-ffi ABI key, else freshly cross-built.  Best-effort — a missing
/// toolchain/target, or a crate that is not wasm-clean, returns `false` (the caller then
/// emits the "no wasm build" notice rather than dying on a bare `E0463`).
/// Whether anything in `<pkg_dir>/native/` is newer than `artifact` — the crate's own
/// hand-written Rust, which no other freshness check looks at (loft#965).
///
/// The sibling walk for GENERATED code (`native_lib.rs::source_newer_than`) covers
/// `*.loft` and `loft.toml` and explicitly SKIPS the `native` directory, and the artifact
/// fingerprint answers a question about the environment (loft-ffi ABI, RUSTFLAGS, codegen
/// version) rather than about the sources.  So between them a `#native` package's own
/// `src/lib.rs` was the one input nothing watched.
///
/// `target/` is skipped: it is the build's own output, and it is always newer.
/// Unreadable metadata answers "newer", so the failure mode is a rebuild, never a reuse
/// of something stale.
pub(crate) fn native_crate_newer_than(pkg_dir: &str, artifact: &std::path::Path) -> bool {
    let Ok(art) = crate::file_access::metadata(artifact).and_then(|m| m.modified()) else {
        return true;
    };
    let mut stack = vec![std::path::PathBuf::from(pkg_dir).join("native")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = crate::file_access::read_dir(&dir) else {
            continue;
        };
        for e in entries {
            if e.file_name() == Some("target") {
                continue;
            }
            // Not followed, as the directory entry answered it: a link is not walked.
            let Ok(meta) = crate::file_access::symlink_metadata(&e) else {
                continue;
            };
            if meta.file_type().is_dir() {
                stack.push(e.os_spelling());
                continue;
            }
            if meta.modified().is_ok_and(|mt| mt > art) {
                return true;
            }
        }
    }
    false
}

/// @PLN21 Phase 3 — a trailing hint naming the package's declared `[native]
/// build-deps` (the system dev packages its cdylib needs to compile), for a
/// failed source build.  Empty when none are declared.
pub(crate) fn build_deps_hint(pkg_dir: &str) -> String {
    let deps = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .map(|m| m.build_deps)
        .unwrap_or_default();
    if deps.is_empty() {
        String::new()
    } else {
        format!(
            " This package needs these dev packages to build: {}.",
            deps.join(", ")
        )
    }
}

/// Resolve the platform-correct shared-library filename from a stem.
#[must_use]
pub fn platform_lib_name(stem: &str) -> String {
    crate::platform::cdylib_file_name(stem)
}

/// The OS a library NAME targets, inferred from its soname/extension form, or
/// `None` when it can't be told (extensionless / unrecognised).  Soname matching,
/// not file-extension parsing: `.so` is matched as a substring because versioned
/// sonames are `.so.N` (which `Path::extension` would read as "N"), and sonames
/// are lowercase by platform convention — so case-sensitive substring checks are
/// correct here.  The single source of truth for runtime-lib platform-scoping.
#[cfg(feature = "native-extensions")]
pub(crate) fn lib_name_target_os(lib: &str) -> Option<&'static str> {
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    if lib.contains(".so") {
        Some("Linux")
    } else if lib.ends_with(".dylib") {
        Some("macOS")
    } else if lib.ends_with(".dll") {
        Some("Windows")
    } else {
        None
    }
}

/// @PLN21 Phase 3 — the first host-applicable `[native] runtime-libs` entry the
/// dynamic linker can't find, if any.  `dlopen` IS the authoritative presence
/// check: a declared lib that loads is present; one that fails is missing, so the
/// package's cdylib (prebuilt OR freshly built) could not load.
///
/// A runtime-lib named for a DIFFERENT OS than the host (e.g. a Linux `libGL.so.1`
/// declared by a library used on macOS) is **skipped, not probed**: it can never
/// load here and is not a host requirement — the host build of the cdylib links
/// its own platform libraries.  Probing it would wrongly hard-fail the whole
/// library on every foreign platform.  Only host-applicable (or unclassifiable)
/// names gate resolution.  Empty `runtime-libs` → no check, no cost.
#[cfg(feature = "native-extensions")]
pub(crate) fn first_missing_runtime_lib(pkg_dir: &str) -> Option<String> {
    let host = host_os_name();
    crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))?
        .runtime_libs
        .into_iter()
        .filter(|lib| lib_name_target_os(lib).is_none_or(|os| os == host))
        // SAFETY: loading runs the library's initialisers; a declared runtime library is meant to load.
        .find(|lib| unsafe { crate::file_access::load_library(lib) }.is_err())
}
