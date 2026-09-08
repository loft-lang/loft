// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! `loft install` orchestration — drives the full install flow
//! described in [PKG_REGISTRY.md § `loft install` flow](../doc/claude/PKG_REGISTRY.md#loft-install-flow).
//!
//! Covers PKG.REG R4 (single-package install), R5 (project-wide
//! install), R6 (`loft update`), R7 (transitive resolution).  The
//! actual CLI plumbing (`loft install` subcommand arg parsing) lives
//! in `main.rs::install_v2`; this module is the engine.
//!
//! Flow:
//!
//! 1. Resolve registry URL (env override or compiled-in default).
//! 2. Fetch `index.json` + `index.json.sig`; verify signature.
//! 3. Parse the index.
//! 4. Resolve the requested package + version (handling
//!    constraints and transitive deps).
//! 5. For each resolved package: download tarball → verify sha256 →
//!    extract to `~/.loft/registry/<pkg>-<version>/`.
//! 6. Write `loft.lock` reflecting the install graph.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};

use crate::lockfile::{self, LockFile, LockedPackage, SCHEMA_VERSION};
use crate::registry_index::{self, RegistryIndex, Version, extract_tarball};
use crate::registry_signing::{self, VerifyResult};

/// Knobs the CLI surface passes through.  Mirrors the flags in
/// PKG_REGISTRY.md § CLI surface.
#[allow(clippy::struct_excessive_bools)] // CLI flag bag; bool fields map directly to `--flag` switches
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Bypass signature verification.  Required when
    /// `TRUSTED_PUBLIC_KEYS` is empty (pre-bootstrap state) AND the
    /// user explicitly opts out; refuses to proceed silently
    /// otherwise.
    pub allow_unsigned: bool,
    /// Force re-fetch of the registry index even if the cache TTL
    /// hasn't expired.  Mirrors `--refresh`.
    pub refresh: bool,
    /// Use cache only; fail if a requested package isn't already
    /// cached or the index is stale.
    pub offline: bool,
    /// Accept prerelease versions when resolving constraints.
    pub allow_prerelease: bool,
    /// Override the lockfile path written by `install_one`.
    /// `None` → default cwd/`loft.lock` (existing behaviour).
    /// `Some(path)` → write/merge into that path instead.
    /// Used by `loft pin <script>` (writes `<script>.loft.lock`
    /// next to the script) and by future project-mode walk-up
    /// resolution (writes to the project root's `loft.lock`).
    pub lock_path: Option<PathBuf>,
    /// Install the package(s) but write NO lockfile.  Set when a resolution
    /// originates INSIDE the registry cache — a transitive dep auto-installed
    /// while parsing an already-cached package (`~/.loft/registry/<pkg>/src`).
    /// There is no consumer project to record against, and the only "project
    /// root" the walk-up finds is the cached dependency's own dir, so the
    /// default would write `loft.lock` INTO the immutable cache — harmless on
    /// Unix (the dir is writable) but an ENOENT that aborts the whole resolution
    /// on Windows.  Skipping the write is correct on every platform: the install
    /// still lands, so `use <dep>` resolves.
    pub skip_lockfile: bool,
}

/// High-level outcome printed back to the user.
#[derive(Debug)]
pub struct InstallReport {
    pub installed: Vec<(String, String)>,
    pub skipped_cached: Vec<(String, String)>,
    /// @PLN21 Phase 5 — per-package native surfacing lines (prebuilt
    /// availability for this host + declared runtime libs), rendered by
    /// `format_report` so a user sees whether `use <pkg>` needs a toolchain.
    pub surface: Vec<String>,
}

/// @PLN21 Phase 4 — opportunistically fetch a prebuilt cdylib for THIS host so
/// the package runs with NO Rust toolchain.  Best-effort: any miss (no entry for
/// the host triple, an fp built against a different loft-ffi, offline, or a
/// download/hash failure) silently leaves only the source, which
/// `auto_build_native` compiles on first use.  On success the cdylib lands at
/// `prebuilt/<triple>/lib<stem>.<ext>` + a `.loft-build-fp` sidecar — exactly
/// where `extensions::resolve_native_lib` looks first (Phase 1).
fn fetch_prebuilt(r: &ResolvedPackage, opts: &InstallOptions) -> bool {
    if opts.offline {
        return false;
    }
    let triple = crate::cache::host_triple();
    let Some(bin) = r.version.binaries.get(&triple) else {
        return false;
    };
    // Only a binary built against THIS loft's loft-ffi ABI is compatible.
    if bin.loft_ffi_fp != Some(crate::cache::loft_ffi_fingerprint()) {
        return false;
    }
    let pkg_dir = registry_index::extract_dir(&r.name, &r.version.semver);
    // The cdylib stem is the package manifest's `[library] native` field.
    let Some(stem) = crate::manifest::read_manifest(&pkg_dir.join("loft.toml").to_string_lossy())
        .and_then(|m| m.native)
        // The stem becomes a FILENAME below, and this manifest came off the network.
        // Same rule, same reason as the package name: see `libscan::is_valid_package_name`.
        .filter(|s| crate::libscan::is_valid_package_name(s))
    else {
        return false;
    };
    let dir = pkg_dir.join("prebuilt").join(&triple);
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let filename = if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    };
    let dest = dir.join(&filename);
    let Ok(bytes) = registry_index::download_tarball(&bin.url, &dest) else {
        return false;
    };
    if crate::integrity::verify_sha256(&bytes, &bin.sha256).is_err() {
        let _ = std::fs::remove_file(&dest);
        return false;
    }
    // Stamp the sidecar so Phase 1's fp-gated resolve accepts the binary.
    crate::cache::write_native_artifact_fingerprint(&dir, crate::cache::loft_ffi_fingerprint());
    true
}

/// @PLN21 Phase 5 — the prebuilt-availability one-liner for a package: given the
/// host triple, the triples that have a published binary, and whether one was
/// just installed.  Pure (no IO) → unit-tested directly.
fn prebuilt_status(host: &str, available: &[String], installed: bool) -> String {
    if installed {
        format!("prebuilt installed for {host} — no Rust toolchain needed")
    } else if available.iter().any(|t| t == host) {
        format!(
            "a {host} prebuilt exists but was built against a different loft-ffi — \
             builds from source on first use"
        )
    } else if available.is_empty() {
        "no prebuilt published — builds from source on first use (needs rustc)".to_string()
    } else {
        format!(
            "no prebuilt for {host} (available: {}) — builds from source on first use (needs rustc)",
            available.join(", ")
        )
    }
}

/// Install a single named package (with optional version
/// constraint).  Drives steps 1-6 of the flow above.
///
/// `constraint`:
/// - `None` → highest non-yanked non-prerelease version.
/// - `Some("0.1.0")` → exact pin.
/// - `Some("^0.1")` etc. → semver constraint.
///
/// # Errors
///
/// Returns a `String` error on any failure point.  Errors are
/// composed end-user-readable; the caller writes them to stderr.
pub fn install_one(
    package_name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
) -> Result<InstallReport, String> {
    let index = load_index(opts)?;
    let mut graph: Vec<ResolvedPackage> = Vec::new();
    resolve_recursive(
        &index,
        package_name,
        constraint,
        opts,
        &held_versions(opts),
        &mut graph,
    )?;
    check_against_lockfile(&graph, opts)?;

    let mut report = InstallReport {
        installed: Vec::new(),
        skipped_cached: Vec::new(),
        surface: Vec::new(),
    };

    for r in &graph {
        let dir = registry_index::extract_dir(&r.name, &r.version.semver);
        if dir.join("loft.toml").exists() {
            report
                .skipped_cached
                .push((r.name.clone(), r.version.semver.clone()));
            continue;
        }
        let tarball_path =
            registry_index::cache_dir().join(format!("{}-{}.tar.gz", r.name, r.version.semver));
        let bytes = if opts.offline {
            return Err(format!(
                "package `{}-{}` not cached; offline mode refuses to fetch",
                r.name, r.version.semver
            ));
        } else {
            // @PLN143 — the one line the registry prints on the resolution path, and it
            // speaks where bytes are actually fetched.  `[registry] resolving <pkg>` used
            // to print BEFORE this, so a warm cache announced work it was not doing —
            // twice per run once a bare `use` re-decides on every parse pass, which is
            // noise on a program that has nothing to report (GOALS.md: loft is noticed in
            // its absence).  Downloading a tarball is worth a line; finding it already
            // extracted is not.
            eprintln!("[registry] downloading {} {}", r.name, r.version.semver);
            registry_index::download_tarball(&r.version.url, &tarball_path)?
        };
        crate::integrity::verify_sha256(&bytes, &r.version.sha256)?;
        extract_tarball(&tarball_path, &registry_index::cache_dir())?;
        // Tarball is consumed — remove it to save space.  The
        // extracted dir is the canonical install.
        let _ = std::fs::remove_file(&tarball_path);
        // @PLN21 Phase 4 — best-effort: grab a host-matching prebuilt cdylib so
        // first use needs no toolchain (silently no-ops when none applies).
        let got_prebuilt = fetch_prebuilt(r, opts);
        // @PLN21 Phase 5 — surface whether `use <pkg>` needs a toolchain, plus
        // any declared runtime system libs.
        let host = crate::cache::host_triple();
        let available: Vec<String> = r.version.binaries.keys().cloned().collect();
        report.surface.push(format!(
            "{} {}: {}",
            r.name,
            r.version.semver,
            prebuilt_status(&host, &available, got_prebuilt)
        ));
        if let Some(m) = crate::manifest::read_manifest(&dir.join("loft.toml").to_string_lossy()) {
            if !m.runtime_libs.is_empty() {
                report.surface.push(format!(
                    "  runtime libraries: {}",
                    m.runtime_libs.join(", ")
                ));
            }
            // @PLN24 arc D — build the package's ANSI-C shim now, at install.
            //
            // It would build on first use anyway (the parser compiles it when it
            // registers the package's C libraries), so this is not about making
            // it work — it is about WHEN the answer arrives. A package needing a
            // C compiler should say so while the user is installing packages and
            // expecting to hear about dependencies, not later inside the first
            // run of their program. Same reasoning as the prebuilt-cdylib fetch
            // three lines up.
            //
            // Best-effort, and deliberately so: a failure here is surfaced and
            // the install still succeeds, because the parser will report it
            // again — with the source position of the `use` — if the package is
            // actually used. Refusing the install would make a missing `cc`
            // block packages the user may never call into.
            if !m.c_shim.is_empty() {
                match crate::c_shim::build(&dir.to_string_lossy(), &m.c_shim) {
                    Ok(so) => report.surface.push(format!(
                        "  C shim built: {}",
                        so.file_name().unwrap_or(so.as_os_str()).to_string_lossy()
                    )),
                    Err(why) => report.surface.push(format!(
                        "  C shim NOT built — {why}; `use {}` will report it again",
                        r.name
                    )),
                }
            }
        }
        report
            .installed
            .push((r.name.clone(), r.version.semver.clone()));
    }

    // A cache-internal resolution installs but records nothing — there is no
    // consumer project here, only the cached dependency whose source triggered
    // this (see `InstallOptions::skip_lockfile`).
    if opts.skip_lockfile {
        return Ok(report);
    }

    // Write lockfile.  When a lockfile already exists (e.g. from a
    // previous `loft install <other>`), MERGE this install's graph
    // into it: new entries overwrite same-named entries, others
    // survive.  This lets `loft install crypto` followed by
    // `loft install random` produce a combined lockfile listing both
    // packages — matches expectations from cargo / npm.
    //
    // Path: `opts.lock_path` if set (used by `loft pin <script>`
    // for the sidecar `<script>.loft.lock`); otherwise cwd's
    // `loft.lock` (default for `loft install`).
    let lock_path = match &opts.lock_path {
        Some(p) => p.clone(),
        None => std::env::current_dir()
            .map_err(|e| format!("cwd: {e}"))?
            .join("loft.lock"),
    };
    let mut lock = match lockfile::read_lockfile(&lock_path) {
        Ok(Some(existing)) => existing,
        _ => lockfile::LockFile {
            schema_version: 1,
            packages: Vec::new(),
        },
    };
    let new_lock = build_lockfile(&graph);
    for new_pkg in new_lock.packages {
        if let Some(existing) = lock.packages.iter_mut().find(|p| p.name == new_pkg.name) {
            *existing = new_pkg;
        } else {
            lock.packages.push(new_pkg);
        }
    }
    lockfile::write_lockfile(&lock_path, &lock).map_err(|e| format!("write lockfile: {e}"))?;

    Ok(report)
}

#[derive(Debug, Clone)]
struct ResolvedPackage {
    name: String,
    version: Version,
}

/// The parsed index plus the one signal a read-only discovery command needs:
/// whether a network fetch was attempted and FAILED, so the index was served
/// from a possibly-stale cache as a fallback.  Distinct from a fresh cache hit
/// or explicit `--offline` (neither means "unreachable").
pub struct LoadedIndex {
    pub index: RegistryIndex,
    pub stale_fallback: bool,
}

/// Fetch, verify, and parse the registry index — the single loader every
/// registry-reading command shares (`install`, `search`, `info`, `update`,
/// `pin`).  Honours `LOFT_REGISTRY_URL`; serves the cached
/// `~/.loft/registry/index.json` when it is fresh (1-hour TTL) or when
/// `opts.offline`, otherwise re-fetches and re-caches.  Signature outcomes are
/// surfaced per `opts.allow_unsigned` (see `verify_or_explain`).
///
/// A failed network fetch is a hard error here (the install path wants fresh
/// data); read-only commands that prefer a stale cache over failing use
/// [`load_index_reporting`].
///
/// # Errors
/// Returns `Err` when the index cannot be fetched or read, fails signature
/// verification (subject to `opts.allow_unsigned`), or is not valid JSON.
pub fn load_index(opts: &InstallOptions) -> Result<RegistryIndex, String> {
    load_index_inner(opts, false).map(|l| l.index)
}

/// Like [`load_index`], but reports cache-vs-fresh AND, when the network fetch
/// fails, falls back to a usable cached index instead of erroring (with
/// `stale_fallback = true`).  Used by `loft search` / `loft info` so discovery
/// still works when the registry is unreachable.
///
/// # Errors
/// Returns `Err` when a network fetch fails and there is no cached index to
/// fall back to, or the index fails signature verification or parsing.
pub fn load_index_reporting(opts: &InstallOptions) -> Result<LoadedIndex, String> {
    load_index_inner(opts, true)
}

fn load_index_inner(
    opts: &InstallOptions,
    fallback_on_fetch_failure: bool,
) -> Result<LoadedIndex, String> {
    let url = registry_index::registry_url();
    let (idx_path, sig_path, _) = registry_index::index_paths();
    let (content_bytes, stale_fallback): (Vec<u8>, bool) = if opts.offline {
        // @PLN143 — verify here too.  This was the one branch of the four that read the
        // cached index and trusted it unchecked, which put the whole signature gate
        // behind a single environment variable: `LOFT_OFFLINE=1` and the bytes are
        // whatever is on disk.  The cache is where a rejected fetch used to linger (see
        // the fetch branch below), so it is not a place trust can be assumed.
        let content = read_cached_index_verified(&idx_path, &sig_path, opts).map_err(|e| {
            e.message(&format!(
                "offline mode: no cached index ({})",
                idx_path.display()
            ))
        })?;
        (content, false)
    } else if opts.refresh || index_stale(&idx_path) {
        match registry_index::fetch_index(&url) {
            Ok(fetched) => {
                // Verify BEFORE caching.  Writing first and checking after left a
                // rejected index in the cache, where it outlived the run that fetched
                // it: the next command read those bytes against the previous, still
                // valid `.sig`, failed the same way, and kept failing — a fetch that
                // was correctly refused turned into a persistent outage that no
                // retry could clear.  Found by pointing `LOFT_REGISTRY_URL` at a
                // local mirror: one rejected fetch, and every registry-touching test
                // failed until the cache was deleted by hand.
                //
                // A signature that is ABSENT falls back to the cached one — that is
                // the offline / bundle-import path (`fetch_index` leaves it empty
                // when there is no `.sig` beside the index) — but the verdict is
                // still reached before anything is written.
                let sig_bytes = if fetched.signature.is_empty() {
                    std::fs::read(&sig_path).unwrap_or_default()
                } else {
                    fetched.signature.clone()
                };
                verify_or_explain(&fetched.content, &sig_bytes, opts)?;
                // Verified: now it is safe to keep.  Both files land atomically,
                // because a plain write truncates first and another process
                // reading this cache mid-refresh would get a half index or the
                // new index beside the old signature (loft#1045).
                registry_index::write_signed_pair(
                    &idx_path,
                    &sig_path,
                    &fetched.content,
                    &fetched.signature,
                )
                .map_err(|e| format!("cache index: {e}"))?;
                (fetched.content, false)
            }
            Err(fetch_err) => {
                // The fetch failed.  Discovery commands fall back to a cached
                // index if one exists; the install path surfaces the error.
                match fallback_on_fetch_failure
                    .then(|| read_cached_index_verified(&idx_path, &sig_path, opts))
                {
                    Some(Ok(content)) => (content, true),
                    // A cache that cannot be READ leaves the fetch failure as the
                    // truer story; one that fails to VERIFY is its own finding and
                    // must not be reported as a network problem.
                    Some(Err(CachedPairError::Verify(msg))) => return Err(msg),
                    Some(Err(CachedPairError::Io(_))) | None => return Err(fetch_err),
                }
            }
        }
    } else {
        let content = read_cached_index_verified(&idx_path, &sig_path, opts)
            .map_err(|e| e.message("read cached index"))?;
        (content, false)
    };
    let text = std::str::from_utf8(&content_bytes)
        .map_err(|e| format!("index is not valid UTF-8: {e}"))?;
    let index = registry_index::parse_index(text)?;
    // The compiler asks for the trigger map mid-parse and must not pay an index
    // parse for it; a command that already holds the parsed index is the cheapest
    // place to keep its sidecar current.
    registry_index::refresh_trigger_sidecar(&index, content_bytes.len() as u64);
    Ok(LoadedIndex {
        index,
        stale_fallback,
    })
}

fn verify_or_explain(content: &[u8], sig: &[u8], opts: &InstallOptions) -> Result<(), String> {
    let result = registry_signing::verify_index(content, sig);
    match result {
        VerifyResult::Valid => Ok(()),
        VerifyResult::NoTrustRoot => {
            if opts.allow_unsigned {
                Ok(())
            } else {
                Err(
                    "registry index unsigned and this loft binary has no embedded trust root; \
                     pass --allow-unsigned to proceed at your own risk, or install a loft \
                     release with the registry trust root embedded"
                        .to_string(),
                )
            }
        }
        VerifyResult::Invalid => Err("registry index signature INVALID — refusing to install; \
             this is a hard failure even with --allow-unsigned (the signature \
             exists but doesn't verify against any known key)"
            .to_string()),
        VerifyResult::MalformedSignature => {
            if opts.allow_unsigned {
                Ok(())
            } else {
                Err("registry index signature is malformed or missing; \
                     pass --allow-unsigned to proceed at your own risk"
                    .to_string())
            }
        }
    }
}

/// Why reading the cached index+signature pair did not produce a usable index:
/// the files could not be READ, or they read fine and the signature did not
/// verify.  The two want different words from the caller — a missing cache is
/// "offline mode: no cached index" on one path and the network's own error on
/// another, while a verification verdict is already a finished sentence.
enum CachedPairError {
    Io(std::io::Error),
    Verify(String),
}

impl CachedPairError {
    /// The message for this failure, with `io_prefix` naming what the caller was
    /// trying to read when the failure was an I/O one.
    fn message(self, io_prefix: &str) -> String {
        match self {
            Self::Io(e) => format!("{io_prefix}: {e}"),
            Self::Verify(msg) => msg,
        }
    }
}

/// Read the cached index against the cached signature, treating a refresh landing
/// underneath as what it is rather than as a bad signature.
///
/// The pair is written as two files and cannot be swapped in one step, so a reader
/// crossing a refresh can pick up the new index beside the old `.sig`.  That
/// combination fails verification, and the message it produces — *the signature
/// exists but doesn't verify against any known key* — is the one a TAMPERED index
/// produces, deliberately un-bypassable even with `--allow-unsigned`.  A consumer
/// hit exactly this: a suite failed hard, the identical command passed minutes
/// later, and the diagnosis pointed at trust roots rather than at the toolchain
/// install three minutes earlier (loft#1045).
///
/// So a rejection is re-read before it is believed.  The signature is an exact
/// oracle for the question being asked: one that verifies over the content it was
/// read with proves both came from the same generation, so a pair accepted here is
/// matched rather than merely plausible.  A signature that genuinely does not
/// verify never comes to agree, and is reported with its own words unchanged —
/// the settle costs it a bounded wait, on a path that ends in an aborted command.
fn read_cached_index_verified(
    idx_path: &Path,
    sig_path: &Path,
    opts: &InstallOptions,
) -> Result<Vec<u8>, CachedPairError> {
    let settled = registry_index::read_signed_pair_settling(idx_path, sig_path, &mut |c, s| {
        verify_or_explain(c, s, opts).is_ok()
    })
    .map_err(CachedPairError::Io)?;
    if !settled.accepted {
        // Not accepted means the last attempt returned Err; re-run it for the text.
        if let Err(msg) = verify_or_explain(&settled.content, &settled.signature, opts) {
            return Err(CachedPairError::Verify(msg));
        }
    }
    Ok(settled.content)
}

fn index_stale(idx_path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(idx_path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(age) = modified.elapsed() else {
        return true;
    };
    age.as_secs() > 60 * 60 // 1-hour TTL per PKG_REGISTRY.md
}

/// The lockfile whose entries this resolution is bound by — @PLN143.
///
/// `lock_path` names the lock that GOVERNS, and `skip_lockfile` says whether this
/// resolution may write it; the two are separate questions, and reading them as one is
/// what let a pinned script's sidecar decide which file loaded while deciding nothing
/// about which version got installed.
///
/// - **Named** → that file, whether or not it may be written. A pinned script's sidecar
///   governs and must not be amended by a run.
/// - **Unnamed, may write** → the cwd's `loft.lock`, which is where `loft install`
///   writes by default and therefore what it is already bound by.
/// - **Unnamed, may not write** → nothing. There is no declaration in force: a bare
///   script, or a dep resolved inside the registry cache with no consumer project.
fn governing_lock_path(opts: &InstallOptions) -> Option<PathBuf> {
    match (&opts.lock_path, opts.skip_lockfile) {
        (Some(p), _) => Some(p.clone()),
        (None, false) => std::env::current_dir().ok().map(|c| c.join("loft.lock")),
        (None, true) => None,
    }
}

/// The options a `use`-driven auto-install runs with, for a program in `scope` — @PLN143.
///
/// One function so the whole posture of that path is stated in one place and testable
/// without a registry:
///
/// - **The index signature is not waived** (arc A). `loft install` keeps its own CLI
///   default, and the asymmetry is the point: waiving is defensible for a verb a person
///   typed, not for the path a bare `use` takes on its own.
/// - **The governing lock is READ, and written only where a declaration may be
///   recorded.** They were one field, and reading them as one is what let a pinned
///   script's sidecar decide which file loaded while deciding nothing about which
///   version was installed.
/// - **A resolution inside the registry cache is bound by neither**: the only
///   declaration above it is the cached dependency's own, and the consumer's constraint
///   reaches it through the root manifest instead.
/// - No `refresh` (the index TTL decides that) and no prereleases — a `use` takes
///   releases.
#[must_use]
pub fn options_for_use(
    scope: &crate::resolution_scope::ResolutionScope,
    in_registry_cache: bool,
    offline: bool,
) -> InstallOptions {
    let write_target = scope.lock_write_target(in_registry_cache);
    InstallOptions {
        allow_unsigned: false,
        refresh: false,
        skip_lockfile: write_target.is_none(),
        lock_path: if in_registry_cache {
            None
        } else {
            scope.governing_lock()
        },
        // `LOFT_OFFLINE=1` makes resolution HERMETIC: a missing package fails fast and
        // deterministically instead of fetching — what a test-spawned fixture (or an
        // air-gapped box) wants.  Mirrors the CLI paths that already honour it.
        offline,
        allow_prerelease: false,
    }
}

/// The constraint a resolution runs under, given what the governing lock pins and what
/// the manifest declares — @PLN143.
///
/// A lock entry is an EXACT version, so it outranks a range: that is what a lockfile IS,
/// the resolved form of the declaration above it. The one case where it does not is a
/// pin the manifest has since excluded — someone edited `^0.1` to `^0.2` and has not
/// re-installed — and answering the stale pin there would honour a declaration that has
/// been overruled by the one it derives from.
///
/// `None` means unconstrained: nothing is declared, so the newest release, which is what
/// a bare script means by `use`.
#[must_use]
pub fn constraint_for(pinned: Option<&str>, declared: Option<&str>) -> Option<String> {
    match (pinned, declared) {
        (Some(v), Some(c)) if !registry_index::satisfies(v, c) => Some(c.to_string()),
        (Some(v), _) => Some(v.to_string()),
        (None, c) => c.map(ToString::to_string),
    }
}

/// What this project already holds, read from the lock that governs it — the input step 6
/// needs to tell a first install from an upgrade.
///
/// Empty where no declaration is in force (see [`governing_lock_path`]), or where the
/// lockfile cannot be read.  Empty means unconstrained, so those paths resolve exactly as
/// they did before.
fn held_versions(opts: &InstallOptions) -> std::collections::BTreeMap<String, String> {
    use std::collections::BTreeMap;
    let Some(lock_path) = governing_lock_path(opts) else {
        return BTreeMap::new();
    };
    match lockfile::read_lockfile(&lock_path) {
        Ok(Some(l)) => l
            .packages
            .into_iter()
            .map(|p| (p.name, p.version))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Refuse to install when the index now serves DIFFERENT bytes for a version this
/// project already locked.
///
/// A lockfile records `name`, `version` and `sha256`.  The version pin was always
/// honoured; the sha256 was written and never read again, which made it a record of
/// something nobody checked — so a version could be re-published under the same number
/// and every locked consumer would silently take the new bytes.  A lockfile that pins a
/// version but not its contents is not a lock.
///
/// Checked before downloading rather than at the hash-verify site: the two hashes are
/// both already in hand, so there is no reason to fetch a tarball first, and refusing
/// early keeps the failure about the disagreement instead of about a download.
///
/// Only the SAME version is compared.  A different version is an upgrade — the point of
/// resolution — and its hash is expected to differ.
fn check_against_lockfile(graph: &[ResolvedPackage], opts: &InstallOptions) -> Result<(), String> {
    let locked = locked_hashes(opts);
    for r in graph {
        let Some((ver, sha)) = locked.get(&r.name) else {
            continue;
        };
        if ver != &r.version.semver || sha.is_empty() {
            continue;
        }
        if !sha.eq_ignore_ascii_case(&r.version.sha256) {
            return Err(format!(
                "`{}` {} does not match your lockfile: it records sha256 {}, the registry \
                 now serves {}.  The same version must always be the same bytes, so this \
                 is a re-publish or a tampered index — not an upgrade.  Delete the \
                 lockfile entry to accept the new bytes deliberately.",
                r.name, r.version.semver, sha, r.version.sha256
            ));
        }
    }
    Ok(())
}

/// Every `name -> (version, sha256)` a lockfile pins, for [`check_against_lockfile`].
fn locked_hashes(opts: &InstallOptions) -> std::collections::BTreeMap<String, (String, String)> {
    use std::collections::BTreeMap;
    // @PLN143 — the same lock `held_versions` reads, from the same rule: a lockfile
    // records the bytes it pinned, so whatever governs the versions governs the hashes.
    // A pinned script's sidecar therefore gets the re-publish check too, and a resolution
    // with no declaration in force is checked against nothing rather than against a stray
    // `loft.lock` in whatever directory the run happened to start in.
    let Some(lock_path) = governing_lock_path(opts) else {
        return BTreeMap::new();
    };
    match lockfile::read_lockfile(&lock_path) {
        Ok(Some(l)) => l
            .packages
            .into_iter()
            .map(|p| (p.name, (p.version, p.sha256)))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// Recursive resolver: pull `name` (and its transitive deps) into
/// `graph`.  Diamond resolution: when a package appears twice via
/// different dep paths, pick the **highest** version satisfying both
/// constraints; conflict otherwise.
fn resolve_recursive(
    index: &RegistryIndex,
    name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
    held: &std::collections::BTreeMap<String, String>,
    graph: &mut Vec<ResolvedPackage>,
) -> Result<(), String> {
    if graph.iter().any(|r| r.name == name) {
        // Already resolved — TODO when conflict detection grows, re-
        // check that the existing pin satisfies the new constraint.
        return Ok(());
    }
    // @PLN78 — the toolchain is in the registry so it can be found and updated, not so
    // it can be installed as a dependency.  Both differ from a library in the same way:
    // `install` unpacks a source tree into the package cache for `use` to resolve
    // against, while the toolchain is the program doing the resolving, and replacing it
    // is `self-update`'s job (atomic rename, a running binary, `verify-self` afterwards).
    //
    // The guard sits here rather than in `install_one` because this is the one point
    // every route passes: the direct ask, a `deps` entry, and the parser's auto-install
    // fallback for `use loft;`.  Without it each arrives at the same confusing end —
    // downloading a source zip and failing to untar it as a library.
    if name == crate::self_update::TOOLCHAIN_PKG {
        return Err(if graph.is_empty() {
            "`loft` is the toolchain, not a library.  To update it: loft self-update".to_string()
        } else {
            format!(
                "`{}` depends on `loft`, which is the toolchain rather than a library.  \
                 A package states the loft it needs with `loft = \"...\"` under `[package]`.",
                graph.last().map_or("a package", |r| r.name.as_str())
            )
        });
    }
    let pkg = index
        .packages
        .get(name)
        .ok_or_else(|| format!("package `{name}` not found in registry"))?;
    let constraint_str = constraint.unwrap_or("*");
    // Step 6: if this project already holds a version, re-resolving is an
    // UPGRADE, and an upgrade must not hand it a release that declared it
    // breaks them.  Nothing held → nothing to be a drop-in for, so the fresh
    // install resolves exactly as before.
    let resolved = registry_index::find_compatible_version(
        pkg,
        constraint_str,
        opts.allow_prerelease,
        held.get(name).map(String::as_str),
    );
    for held_back in &resolved.withheld {
        eprintln!(
            "note: `{name}` {} is available but declares a break past the {} you hold \
             (api_compatible_with = {}); staying compatible.  Ask for it by version to take it.",
            held_back.semver,
            held.get(name).map_or("?", String::as_str),
            held_back.api_compatible_with.as_deref().unwrap_or("?"),
        );
    }
    let version = resolved
        .best
        .ok_or_else(|| {
            // A yanked version stays LISTED in `versions` (PKG_REGISTRY.md § Yanking keeps
            // it so a `loft.lock` pin still resolves), so listing the keys bare tells the
            // reader "0.1.0 is available" in the same breath as refusing a constraint
            // 0.1.0 plainly satisfies.  Marking them is the difference between a message
            // that reads as a resolver bug and one that names the cause — and when the
            // yank IS the cause, saying so outright, because that reader's next move is
            // to raise their floor rather than to re-read the constraint.
            let yanked: std::collections::HashSet<&str> =
                pkg.yanked.iter().map(String::as_str).collect();
            let listed = pkg
                .versions
                .keys()
                .map(|v| {
                    if yanked.contains(v.as_str()) {
                        format!("{v} (yanked)")
                    } else {
                        v.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let blocked: Vec<&str> = pkg
                .versions
                .keys()
                .filter(|v| yanked.contains(v.as_str()))
                .filter(|v| registry_index::satisfies(v, constraint_str))
                .map(String::as_str)
                .collect();
            let because = if blocked.is_empty() {
                String::new()
            } else {
                format!(
                    " — every version that matches is yanked ({}); \
                     raise the constraint, or ask for one by exact version to take it anyway",
                    blocked.join(", ")
                )
            };
            format!(
                "no version of `{name}` satisfies constraint `{constraint_str}` \
                 (available: {listed}){because}"
            )
        })?
        .clone();

    let dep_pairs: Vec<(String, String)> = version
        .deps
        .iter()
        .map(|(n, c)| (n.clone(), c.clone()))
        .collect();
    graph.push(ResolvedPackage {
        name: name.to_string(),
        version,
    });
    for (dep_name, dep_constraint) in dep_pairs {
        resolve_recursive(index, &dep_name, Some(&dep_constraint), opts, held, graph)?;
    }
    Ok(())
}

fn build_lockfile(graph: &[ResolvedPackage]) -> LockFile {
    let mut lock = LockFile {
        schema_version: SCHEMA_VERSION,
        packages: Vec::with_capacity(graph.len()),
    };
    for r in graph {
        lock.packages.push(LockedPackage {
            name: r.name.clone(),
            version: r.version.semver.clone(),
            url: r.version.url.clone(),
            sha256: r.version.sha256.clone(),
            source: "registry".to_string(),
            deps: r.version.deps.keys().cloned().collect(),
        });
    }
    lock
}

/// Auto-install fallback fired by the parser's `use X;` resolution
/// chain (@PLAN12 Phase 6.6).
///
/// Behaviour:
/// - Loads the registry index (uses cached, refreshes if TTL stale).
/// - If `name` is in the catalog, installs the latest active version
///   (same machinery as `loft install <name>`).
/// - Returns `Ok(Some(report))` on a successful install, `Ok(None)`
///   when `name` is NOT in the catalog (so the parser can fall
///   through to remaining resolution strategies), or `Err` on a real
///   failure (network down on a cold cache, signature mismatch,
///   etc.).
///
/// The caller (parser's `probe_auto_install`) decides whether to
/// announce or stay silent based on the return value.
///
/// # Errors
///
/// Surfaces any error from `install_one` — network failure, sig
/// mismatch, tarball corruption, lockfile write failure.
/// `constraint` is the root project's declared version requirement for `name`
/// (e.g. `Some("=0.1.0")`), threaded through so a source-level `use` honours the
/// consumer's pin instead of always resolving the latest release.  `None` keeps
/// the historical behaviour (newest satisfying `*`).
pub fn auto_install_if_in_catalog(
    name: &str,
    constraint: Option<&str>,
    opts: &InstallOptions,
) -> Result<Option<InstallReport>, String> {
    // Load the index FIRST so we can check membership without
    // committing to a network fetch for the tarball.  load_index
    // honours `opts.offline` — if true, only uses cached index.
    let index = load_index(opts)?;
    // The toolchain is in the index but is not a library, so it is not something a bare
    // `use` should ever pull in.  Falling through (rather than erroring) is what keeps
    // publishing the entry from changing the meaning of an existing program: before the
    // entry existed `use loft;` was a miss here and went on to the parser's remaining
    // resolution strategies, and it still does.
    if name == crate::self_update::TOOLCHAIN_PKG || !index.packages.contains_key(name) {
        return Ok(None);
    }
    let report = install_one(name, constraint, opts)?;
    for (n, v) in &report.installed {
        eprintln!("[registry] installed {n} {v}");
    }
    Ok(Some(report))
}

/// The newest release of every package the CACHED index knows — @PLN143 arc E.
///
/// The input to the "your pin has fallen behind" notice, and every property of it follows
/// from what that notice is allowed to cost:
///
/// - **Cache only, never a fetch.** The index already refreshes on a 1-hour TTL with a
///   conditional GET; an advisory line may not add a network round trip to a program run.
///   No cached index means an empty map, which means silence.
/// - **`allow_unsigned`**, the posture every read-only registry command takes
///   (`loft search`, `loft info`, `loft api --registry`): a MISSING signature is
///   tolerated, an INVALID one still hard-fails — and here that failure degrades to an
///   empty map, so tampered bytes produce silence rather than a message. Nothing is
///   installed from this, and the cure it prints goes through the verifying path.
/// - The version is picked by [`registry_index::find_best_version`], so "newest" means
///   exactly what it means everywhere else — yanked skipped, prerelease skipped — rather
///   than a second rule that could drift from the resolver's.
#[must_use]
pub fn newest_cached_releases() -> std::collections::BTreeMap<String, String> {
    let opts = InstallOptions {
        allow_unsigned: true,
        offline: true,
        ..Default::default()
    };
    let Ok(index) = load_index(&opts) else {
        return std::collections::BTreeMap::new();
    };
    index
        .packages
        .iter()
        .filter_map(|(name, pkg)| {
            registry_index::find_best_version(pkg, "*", false)
                .map(|v| (name.clone(), v.semver.clone()))
        })
        .collect()
}

/// Render a human-readable summary of an install.
#[must_use]
pub fn format_report(report: &InstallReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if report.installed.is_empty() && report.skipped_cached.is_empty() {
        return "Nothing to do.\n".to_string();
    }
    if !report.installed.is_empty() {
        let _ = writeln!(out, "Installed:");
        for (n, v) in &report.installed {
            let _ = writeln!(out, "  {n} {v}");
        }
    }
    if !report.skipped_cached.is_empty() {
        let _ = writeln!(out, "Already cached (skipped):");
        for (n, v) in &report.skipped_cached {
            let _ = writeln!(out, "  {n} {v}");
        }
    }
    // @PLN21 Phase 5 — per-package native surfacing (prebuilt availability +
    // declared runtime libs), so a user sees whether `use <pkg>` needs a toolchain.
    if !report.surface.is_empty() {
        let _ = writeln!(out, "Native:");
        for line in &report.surface {
            let _ = writeln!(out, "  {line}");
        }
    }
    let total = report.installed.len() + report.skipped_cached.len();
    let _ = writeln!(out, "{total} package(s) total");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_index::{Package, RegistryIndex};
    use std::collections::BTreeMap;

    /// Build a minimal `Version` with placeholder URL/sha/size — the
    /// resolver doesn't touch the network-side fields.
    fn ver(semver: &str, deps: &[(&str, &str)]) -> Version {
        let mut d = BTreeMap::new();
        for (n, c) in deps {
            d.insert((*n).to_string(), (*c).to_string());
        }
        Version {
            semver: semver.to_string(),
            url: format!("https://example.com/{semver}.tar.gz"),
            sha256: "0".repeat(64),
            size: 1,
            loft: ">=0.8".to_string(),
            api_compatible_with: None,
            data_compatible_with: None,
            deps: d,
            conflicts: vec![],
            replaces: vec![],
            provides: vec![],
            triggers: vec![],
            binaries: BTreeMap::new(),
            api: vec![],
            prerelease: false,
            published: "p".to_string(),
        }
    }

    fn pkg(name: &str, versions: Vec<Version>) -> Package {
        let mut vmap = BTreeMap::new();
        for v in versions {
            vmap.insert(v.semver.clone(), v);
        }
        Package {
            name: name.to_string(),
            description: None,
            homepage: None,
            categories: vec![],
            yanked: vec![],
            versions: vmap,
        }
    }

    fn index(pkgs: Vec<Package>) -> RegistryIndex {
        let mut pmap = BTreeMap::new();
        for p in pkgs {
            pmap.insert(p.name.clone(), p);
        }
        RegistryIndex {
            schema_version: 1,
            updated: "now".to_string(),
            packages: pmap,
            skipped: Vec::new(),
        }
    }

    fn opts() -> InstallOptions {
        InstallOptions {
            allow_unsigned: true,
            refresh: false,
            offline: false,
            allow_prerelease: false,
            skip_lockfile: false,
            lock_path: None,
        }
    }

    // ── resolve_recursive — linear / diamond / cycle / missing ──

    #[test]
    fn resolves_linear_deps_in_order() {
        let idx = index(vec![
            pkg("a", vec![ver("0.1.0", &[("b", "^0.1")])]),
            pkg("b", vec![ver("0.1.0", &[("c", "^0.1")])]),
            pkg("c", vec![ver("0.1.0", &[])]),
        ]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        let names: Vec<&str> = graph.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn resolves_diamond_dedups_shared_node() {
        let idx = index(vec![
            pkg("a", vec![ver("0.1.0", &[("b", "^0.1"), ("c", "^0.1")])]),
            pkg("b", vec![ver("0.1.0", &[("d", "^0.1")])]),
            pkg("c", vec![ver("0.1.0", &[("d", "^0.1")])]),
            pkg("d", vec![ver("0.1.0", &[])]),
        ]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        let names: Vec<&str> = graph.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "d", "c"]);
        // Each package appears exactly once.
        let unique: std::collections::HashSet<&&str> = names.iter().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn handles_cycle_without_infinite_loop() {
        // a → b → a — pathological but mustn't hang.
        let idx = index(vec![
            pkg("a", vec![ver("0.1.0", &[("b", "^0.1")])]),
            pkg("b", vec![ver("0.1.0", &[("a", "^0.1")])]),
        ]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        let names: Vec<&str> = graph.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn rejects_missing_transitive_dep() {
        let idx = index(vec![pkg("a", vec![ver("0.1.0", &[("b", "^0.1")])])]);
        let mut graph = Vec::new();
        let err = resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph)
            .expect_err("should fail on missing transitive");
        assert!(err.contains("`b` not found"), "msg: {err}");
    }

    #[test]
    fn rejects_unsatisfiable_constraint() {
        let idx = index(vec![pkg("a", vec![ver("0.1.0", &[])])]);
        let mut graph = Vec::new();
        let err = resolve_recursive(
            &idx,
            "a",
            Some("^0.2"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .expect_err("should fail on unsatisfiable constraint");
        assert!(
            err.contains("satisfies constraint") && err.contains("^0.2"),
            "msg: {err}"
        );
        assert!(err.contains("0.1.0"), "msg: {err}");
        // Negative control for the yank clause below: nothing here is yanked, so the
        // message must NOT claim a yank is why resolution failed.
        assert!(!err.contains("yanked"), "msg: {err}");
    }

    /// A yanked version stays LISTED, so the failure message has to say which entries
    /// the resolver refused — otherwise it names `0.1.0` as available in the same breath
    /// as refusing a constraint `0.1.0` satisfies, and reads as a resolver bug.
    /// (Met live: yanking `imaging` 0.1.0 for loft#1448 made `^0.1.0` fail with a list
    /// that included the version it had just skipped.)
    #[test]
    fn an_unsatisfiable_constraint_names_the_yank_that_caused_it() {
        let mut p = pkg("a", vec![ver("0.1.0", &[]), ver("0.3.0", &[])]);
        p.yanked = vec!["0.1.0".to_string()];
        let idx = index(vec![p]);
        let mut graph = Vec::new();
        let err = resolve_recursive(
            &idx,
            "a",
            Some("^0.1.0"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .expect_err("the only matching version is yanked");
        assert!(err.contains("0.1.0 (yanked)"), "msg: {err}");
        assert!(
            err.contains("every version that matches is yanked"),
            "msg: {err}"
        );
        // The unyanked sibling is listed plainly — the marker is per version, not a
        // banner over the whole package.
        assert!(
            err.contains("0.3.0") && !err.contains("0.3.0 (yanked)"),
            "msg: {err}"
        );
    }

    /// The clause is about the CAUSE, so a failure the yank did not cause must not carry
    /// it — a yanked version that could never have matched is listed, and nothing more.
    #[test]
    fn a_yank_that_did_not_cause_the_failure_is_not_blamed_for_it() {
        let mut p = pkg("a", vec![ver("0.1.0", &[]), ver("0.3.0", &[])]);
        p.yanked = vec!["0.1.0".to_string()];
        let idx = index(vec![p]);
        let mut graph = Vec::new();
        let err = resolve_recursive(
            &idx,
            "a",
            Some("^0.9"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .expect_err("no version reaches 0.9");
        assert!(err.contains("0.1.0 (yanked)"), "msg: {err}");
        assert!(
            !err.contains("every version that matches is yanked"),
            "msg: {err}"
        );
    }

    #[test]
    fn rejects_missing_root_package() {
        let idx = index(vec![]);
        let mut graph = Vec::new();
        let err = resolve_recursive(
            &idx,
            "nope",
            None,
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .expect_err("should fail on unknown package");
        assert!(err.contains("`nope` not found"), "msg: {err}");
    }

    #[test]
    fn resolves_highest_version_by_default() {
        let idx = index(vec![pkg(
            "a",
            vec![ver("0.1.0", &[]), ver("0.1.5", &[]), ver("0.1.2", &[])],
        )]);
        let mut graph = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut graph).unwrap();
        assert_eq!(graph[0].version.semver, "0.1.5");
    }

    #[test]
    fn exact_pin_overrides_highest() {
        // An exact (`=`) constraint pins the OLDER version even when a newer one
        // exists — the feature behind `glb = "=0.1.0"`: pinning is an option, not
        // "always resolve latest".  `auto_install_if_in_catalog` threads this same
        // constraint through from the root project's manifest.
        let idx = index(vec![pkg(
            "a",
            vec![ver("0.1.0", &[]), ver("0.1.1", &[]), ver("0.2.0", &[])],
        )]);
        let mut graph = Vec::new();
        resolve_recursive(
            &idx,
            "a",
            Some("=0.1.0"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .unwrap();
        assert_eq!(graph[0].version.semver, "0.1.0");
        // Sanity: without the pin the newest wins.
        let mut g2 = Vec::new();
        resolve_recursive(&idx, "a", None, &opts(), &BTreeMap::default(), &mut g2).unwrap();
        assert_eq!(g2[0].version.semver, "0.2.0");
    }

    #[test]
    fn resolves_specific_pinned_version() {
        let idx = index(vec![pkg("a", vec![ver("0.1.0", &[]), ver("0.1.5", &[])])]);
        let mut graph = Vec::new();
        resolve_recursive(
            &idx,
            "a",
            Some("0.1.0"),
            &opts(),
            &BTreeMap::default(),
            &mut graph,
        )
        .unwrap();
        assert_eq!(graph[0].version.semver, "0.1.0");
    }

    #[test]
    fn build_lockfile_preserves_order() {
        let v0_1_0 = Version {
            semver: "0.1.0".to_string(),
            url: "u".to_string(),
            sha256: "s".to_string(),
            size: 1,
            loft: ">=0.8".to_string(),
            api_compatible_with: None,
            data_compatible_with: None,
            deps: BTreeMap::new(),
            conflicts: vec![],
            replaces: vec![],
            provides: vec![],
            triggers: vec![],
            binaries: BTreeMap::new(),
            api: vec![],
            prerelease: false,
            published: "p".to_string(),
        };
        let graph = vec![
            ResolvedPackage {
                name: "crypto".to_string(),
                version: v0_1_0.clone(),
            },
            ResolvedPackage {
                name: "web".to_string(),
                version: v0_1_0,
            },
        ];
        let lock = build_lockfile(&graph);
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.packages[0].name, "crypto");
        assert_eq!(lock.packages[1].name, "web");
        assert_eq!(lock.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn format_report_empty() {
        let report = InstallReport {
            installed: vec![],
            skipped_cached: vec![],
            surface: vec![],
        };
        let s = format_report(&report);
        assert!(s.starts_with("Nothing to do"));
    }

    #[test]
    fn format_report_mixed() {
        let report = InstallReport {
            installed: vec![("crypto".to_string(), "0.1.0".to_string())],
            skipped_cached: vec![("web".to_string(), "0.1.0".to_string())],
            surface: vec![],
        };
        let s = format_report(&report);
        assert!(s.contains("Installed:"));
        assert!(s.contains("crypto 0.1.0"));
        assert!(s.contains("Already cached"));
        assert!(s.contains("web 0.1.0"));
        assert!(s.contains("2 package(s) total"));
    }

    // @PLN21 Phase 5 — the prebuilt-availability one-liner across its branches.
    #[test]
    fn prebuilt_status_branches() {
        use super::prebuilt_status;
        let host = "x86_64-unknown-linux-gnu";
        assert!(prebuilt_status(host, &[], true).contains("no Rust toolchain needed"));
        assert!(prebuilt_status(host, &[], false).contains("no prebuilt published"));
        let other = vec!["aarch64-apple-darwin".to_string()];
        assert!(prebuilt_status(host, &other, false).contains("available: aarch64-apple-darwin"));
        let same = vec![host.to_string()];
        assert!(prebuilt_status(host, &same, false).contains("different loft-ffi"));
    }

    /// @PLN78 — the toolchain lives in the registry so it can be found and updated,
    /// which makes `install` a route it must not travel.  Checked at the resolver
    /// rather than the CLI because a `deps` entry and the parser's `use` fallback
    /// reach it without going through the CLI at all.
    #[test]
    fn the_toolchain_is_not_installable_as_a_package() {
        let index = crate::registry_index::parse_index(
            r#"{"schema_version": 1, "updated": "2026-07-31T00:00:00Z", "packages": {
                 "loft": {"description": "toolchain", "categories": [], "yanked": [],
                   "versions": {"2026.7.2": {"url": "u", "sha256": "s", "size": 1,
                     "loft": ">=0", "published": "2026-07-31T00:00:00Z"}}},
                 "widget": {"description": "a library", "categories": [], "yanked": [],
                   "versions": {"0.1.0": {"url": "u", "sha256": "s", "size": 1,
                     "loft": ">=0", "published": "2026-07-31T00:00:00Z",
                     "deps": {"loft": "*"}}}}}}"#,
        )
        .expect("fixture index parses");
        let opts = InstallOptions::default();
        let held = std::collections::BTreeMap::new();

        // Asked for directly: name the command that DOES update loft.
        let mut graph = Vec::new();
        let err = resolve_recursive(&index, "loft", None, &opts, &held, &mut graph)
            .expect_err("`loft install loft` must not resolve");
        assert!(err.contains("self-update"), "must route the user: {err}");

        // Reached as a dependency: name the package that declared it, since the user
        // did not ask for `loft` and cannot otherwise tell where it came from.
        let mut graph = Vec::new();
        let err = resolve_recursive(&index, "widget", None, &opts, &held, &mut graph)
            .expect_err("a dependency on the toolchain must not resolve");
        assert!(
            err.contains("widget"),
            "must name the depending package: {err}"
        );

        // Non-vacuity: an ordinary library still resolves through the same call.
        let mut graph = Vec::new();
        let index_no_dep = crate::registry_index::parse_index(
            r#"{"schema_version": 1, "updated": "2026-07-31T00:00:00Z", "packages": {
                 "widget": {"description": "a library", "categories": [], "yanked": [],
                   "versions": {"0.1.0": {"url": "u", "sha256": "s", "size": 1,
                     "loft": ">=0", "published": "2026-07-31T00:00:00Z"}}}}}"#,
        )
        .unwrap();
        resolve_recursive(&index_no_dep, "widget", None, &opts, &held, &mut graph)
            .expect("an ordinary package must still resolve");
        assert_eq!(graph.len(), 1);
    }

    /// @PLN78 — a lockfile pins bytes, not just a version number.
    ///
    /// The hash was recorded from the first release and never compared again, so a
    /// re-published version reached every locked consumer silently.  These cases are
    /// the whole contract: same version + same hash passes, same version + different
    /// hash refuses, and a DIFFERENT version is an upgrade whose hash is meant to
    /// differ -- get that last one wrong and the check blocks every upgrade instead.
    #[test]
    fn a_lockfile_pins_the_bytes_not_only_the_version() {
        fn version_with(semver: &str, sha: &str) -> Version {
            Version {
                semver: semver.to_string(),
                url: "u".to_string(),
                sha256: sha.to_string(),
                size: 1,
                loft: ">=0".to_string(),
                api_compatible_with: None,
                data_compatible_with: None,
                deps: std::collections::BTreeMap::new(),
                conflicts: Vec::new(),
                replaces: Vec::new(),
                provides: Vec::new(),
                triggers: Vec::new(),
                binaries: std::collections::BTreeMap::new(),
                api: Vec::new(),
                prerelease: false,
                published: "2026-07-31T00:00:00Z".to_string(),
            }
        }
        let dir = std::env::temp_dir().join("loft-lockfile-pin-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lock_path = dir.join("loft.lock");
        let opts = InstallOptions {
            lock_path: Some(lock_path.clone()),
            ..InstallOptions::default()
        };
        lockfile::write_lockfile(
            &lock_path,
            &LockFile {
                schema_version: SCHEMA_VERSION,
                packages: vec![LockedPackage {
                    name: "widget".to_string(),
                    version: "0.1.0".to_string(),
                    url: "u".to_string(),
                    sha256: "aaaa".to_string(),
                    source: "registry".to_string(),
                    deps: Vec::new(),
                }],
            },
        )
        .unwrap();

        let same = |sha: &str, semver: &str| {
            vec![ResolvedPackage {
                name: "widget".to_string(),
                version: version_with(semver, sha),
            }]
        };
        // Control first: unchanged bytes must pass, or the two rejections below prove
        // nothing but that the check refuses everything.
        assert!(check_against_lockfile(&same("aaaa", "0.1.0"), &opts).is_ok());
        // Same version, different bytes -- a re-publish or a tampered index.
        let err = check_against_lockfile(&same("bbbb", "0.1.0"), &opts)
            .expect_err("a re-published version must be refused");
        assert!(
            err.contains("same version must always be the same bytes"),
            "{err}"
        );
        // A genuine upgrade is expected to hash differently.
        assert!(
            check_against_lockfile(&same("bbbb", "0.2.0"), &opts).is_ok(),
            "an upgrade must not be mistaken for tampering"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// @PLN143 — which lockfile a resolution is BOUND by, which is not always the one it
    /// may write. Reading those as one question is what let a pinned script's sidecar
    /// decide which file loaded while deciding nothing about which version was installed.
    #[test]
    fn the_lock_that_governs_is_not_the_lock_that_is_written() {
        let named = InstallOptions {
            lock_path: Some(PathBuf::from("/s.loft.lock")),
            skip_lockfile: true,
            ..Default::default()
        };
        assert_eq!(
            governing_lock_path(&named),
            Some(PathBuf::from("/s.loft.lock")),
            "a sidecar governs even though a run may not amend it"
        );
        let project = InstallOptions {
            lock_path: Some(PathBuf::from("/proj/loft.lock")),
            ..Default::default()
        };
        assert_eq!(
            governing_lock_path(&project),
            Some(PathBuf::from("/proj/loft.lock"))
        );
        let nothing_declared = InstallOptions {
            lock_path: None,
            skip_lockfile: true,
            ..Default::default()
        };
        assert_eq!(
            governing_lock_path(&nothing_declared),
            None,
            "a bare script (or a dep resolved inside the cache) is bound by nothing"
        );
        // The `loft install` default: unnamed, and it writes the cwd's lock — so that is
        // the file it is already bound by.
        let cli = InstallOptions::default();
        assert_eq!(
            governing_lock_path(&cli),
            std::env::current_dir().ok().map(|c| c.join("loft.lock"))
        );
    }

    /// @PLN143 — a lock entry is the resolved form of the declaration above it, so it
    /// outranks a range; a pin the manifest has SINCE excluded does not, because it has
    /// been overruled by the thing it derives from.
    #[test]
    fn a_pin_outranks_a_range_unless_the_manifest_excluded_it() {
        assert_eq!(
            constraint_for(Some("0.1.0"), Some("^0.1")).as_deref(),
            Some("0.1.0"),
            "the exact pin, not the range around it"
        );
        assert_eq!(
            constraint_for(Some("0.1.0"), None).as_deref(),
            Some("0.1.0"),
            "a pinned script has no manifest, and its sidecar still governs"
        );
        assert_eq!(
            constraint_for(Some("0.1.0"), Some("^0.2")).as_deref(),
            Some("^0.2"),
            "a stale lock loses to the declaration it derives from"
        );
        assert_eq!(
            constraint_for(None, Some("^0.2")).as_deref(),
            Some("^0.2"),
            "nothing pinned yet — the manifest decides"
        );
        assert_eq!(
            constraint_for(None, None),
            None,
            "nothing declared at all is the bare script: the newest release"
        );
    }

    /// @PLN143 — the posture of the `use` path, per scope: what it reads, what it may
    /// write, and the signature it never waives.
    #[test]
    fn the_options_a_use_driven_install_runs_with() {
        use crate::resolution_scope::ResolutionScope;
        let sidecar = PathBuf::from("/scripts/s.loft.lock");
        let pinned = options_for_use(
            &ResolutionScope::PinnedScript(sidecar.clone()),
            false,
            false,
        );
        assert_eq!(
            (pinned.lock_path.as_ref(), pinned.skip_lockfile),
            (Some(&sidecar), true),
            "a sidecar governs the install and is not amended by it"
        );

        let root = PathBuf::from("/proj");
        let package = options_for_use(&ResolutionScope::Package(root.clone()), false, false);
        assert_eq!(
            (package.lock_path.as_ref(), package.skip_lockfile),
            (Some(&root.join("loft.lock")), false),
            "a package reads and records the lock beside its manifest"
        );

        let bare = options_for_use(&ResolutionScope::Bare, false, false);
        assert_eq!(
            (bare.lock_path.as_ref(), bare.skip_lockfile),
            (None, true),
            "nothing declared: bound by nothing, and it declares nothing in return"
        );

        let cached = options_for_use(&ResolutionScope::Package(root), true, false);
        assert_eq!(
            (cached.lock_path.as_ref(), cached.skip_lockfile),
            (None, true),
            "a dep resolved inside the cache has no consumer project on either side"
        );

        for o in [&pinned, &package, &bare, &cached] {
            assert!(
                !o.allow_unsigned,
                "the `use` path never waives the index signature (@PLN143 arc A)"
            );
            assert!(!o.allow_prerelease, "a `use` takes releases");
        }
        assert!(options_for_use(&ResolutionScope::Bare, false, true).offline);
    }
}
