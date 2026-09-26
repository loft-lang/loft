// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

// Included by BOTH the library crate (where install.rs + the search/info
// CLI subcommands use the full API) AND the binary crate (where
// parser/mod.rs::probe_registry_installed routes through lockfile only).
// `allow(dead_code)` silences the binary's view; the lib uses every
// symbol so the attribute is a no-op there.
#![allow(dead_code)]

//! Parsed `registry.json` (the JSON-format file-based registry index
//! described in [PKG_REGISTRY.md](../doc/claude/PKG_REGISTRY.md)).
//!
//! PKG.REG R4 scaffolding.  This module owns:
//!
//! - Schema typestate (`RegistryIndex`, `Package`, `Version`).
//! - Index parsing from JSON via `crate::json::parse`.
//! - Version-constraint resolution (`find_best_version`).
//! - Local cache paths (`cache_dir`, `index_paths`, `extract_dir`).
//! - HTTPS fetcher for the index + the per-tarball download (uses
//!   `ureq`, gated on the `registry` feature).
//!
//! What it does NOT own (R5+):
//!
//! - Driving the install flow (`loft install <name>` → calls into
//!   this module then writes `loft.lock`).
//! - Transitive resolution.
//! - The `loft.lock` reader/writer (lives in `crate::lockfile`).
//! - Signature verification of the index (lives in
//!   `crate::registry_signing`).

#![cfg(feature = "registry")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::json::{Parsed, parse as parse_json};

// ── Schema ────────────────────────────────────────────────────────

/// Parsed `registry.json`.
#[derive(Debug, Clone)]
pub struct RegistryIndex {
    pub schema_version: u32,
    pub updated: String,
    pub packages: BTreeMap<String, Package>,
    /// @PLN78 step 1 — packages this index carries that could not be parsed, one
    /// message each.  They are SKIPPED rather than fatal (see [`parse_index`]); a
    /// caller that must be strict — a publishing check, which should refuse to sign
    /// a malformed index — reads this and fails on a non-empty list.
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub categories: Vec<String>,
    pub yanked: Vec<String>,
    pub versions: BTreeMap<String, Version>,
}

#[derive(Debug, Clone)]
pub struct Version {
    pub semver: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub loft: String,
    /// The oldest release of this package that THIS version is still a drop-in
    /// for — mirrored from its `loft.toml` at publish time so a resolver can
    /// read a release's promise without downloading and unpacking its tarball.
    ///
    /// `None` for versions published before the compatibility contract existed,
    /// and that absence is load-bearing: a version that declares nothing has
    /// promised nothing, so resolution must treat it exactly as it did before.
    /// Design: `doc/claude/plans/library-compat-contract/README.md`.
    pub api_compatible_with: Option<String>,
    /// The oldest release whose stored / wire data this version still reads.
    /// Recorded for a consumer to read; resolution does not act on it, because
    /// a data break is not fixed by choosing a different version — the old data
    /// still needs migrating either way.
    pub data_compatible_with: Option<String>,
    pub deps: BTreeMap<String, String>,
    /// Schema slot — resolver-side support deferred.
    pub conflicts: Vec<String>,
    /// Schema slot — resolver-side support deferred.
    pub replaces: Vec<String>,
    /// Schema slot — resolver-side support deferred.
    pub provides: Vec<String>,
    /// Tier-1 lazy-load triggers — text-method names as `"name:receiver"`
    /// (e.g. `"matches:text"`), derived from the package source at publish time
    /// so a CONSUMER's resolver can map `obj.method()` to this package without
    /// having the source.  Produced by `crate::triggers::derive_triggers`.
    pub triggers: Vec<String>,
    /// Schema slot — pre-built distribution deferred.
    pub binaries: BTreeMap<String, BinaryEntry>,
    /// Function-level API surface — one `ApiItem` (signature + one-line doc) per
    /// `pub` item, derived from THIS version's source at publish time (the same
    /// `parse_pkg_api` extractor `loft api` uses) so `loft search` can answer a
    /// consumer's *"is there a function that does X, and how do I call it?"*
    /// against the callable surface WITHOUT the source.  Optional + per-`Version`
    /// (an old pin still describes the functions it actually shipped); the
    /// registry CI re-derives it from source so it cannot drift.  Empty for
    /// indexes published before this field existed.
    pub api: Vec<ApiItem>,
    pub prerelease: bool,
    pub published: String,
}

#[derive(Debug, Clone)]
pub struct BinaryEntry {
    pub url: String,
    pub sha256: String,
    /// @PLN21 Phase 4 — the `loft-ffi` fingerprint (`cache::loft_ffi_fingerprint`)
    /// this prebuilt cdylib was built against.  `loft install` downloads the
    /// binary only when it equals the host loft's fp (a cdylib built against a
    /// different loft-ffi ABI is incompatible) — pre-validation BEFORE the
    /// download.  Stored as a string in the index to avoid u64 JSON precision
    /// loss.  `None` (absent) → skip the prebuilt, fall to source build.
    pub loft_ffi_fp: Option<u64>,
    /// @PLN78 — sha256 of the bundle's `SHA256SUMS`, so an INSTALLED tree can be
    /// checked against the signed index.
    ///
    /// `sha256` above covers the zip, which is verifiable exactly once: at download.
    /// What a user then runs is an unpacked directory, and its manifest ships inside
    /// the bundle it describes — so on its own it proves the installation is undamaged
    /// and cannot prove it is ours.  Naming the manifest's digest here anchors the
    /// whole installation (the binary, and the `default/*.loft` that actually get
    /// loaded) to the one signature we publish, through the one manifest.
    ///
    /// `None` for entries published before this field existed, and that absence is
    /// load-bearing: it means "not anchored", which `verify-self` reports rather than
    /// treating as a pass.
    pub manifest_sha256: Option<String>,
}

/// One function-level API entry: a `pub` item's signature plus a one-line doc
/// summary.  The full multi-line doc stays available via `loft api <pkg>`; only
/// this summary lives in the index, keeping it lean.
#[derive(Debug, Clone)]
pub struct ApiItem {
    pub sig: String,
    pub doc: String,
}

// ── Parsing ───────────────────────────────────────────────────────

/// Parse a `registry.json` document into the strongly-typed schema.
///
/// # Errors
///
/// Returns a `String` error on malformed JSON, unsupported
/// schema_version, or missing required fields.  Schema-violation
/// errors are explicit ("missing `<field>` for package X version Y")
/// rather than generic parse errors, because publishers debugging a
/// rejected PR need to know exactly which line to fix.
pub fn parse_index(content: &str) -> Result<RegistryIndex, String> {
    let parsed = parse_json(content).map_err(|e| format!("JSON parse error: {e:?}"))?;
    let Parsed::Object(root) = parsed else {
        return Err("registry.json: top-level must be an object".to_string());
    };
    let mut schema_version: Option<u32> = None;
    let mut updated: Option<String> = None;
    let mut packages: BTreeMap<String, Package> = BTreeMap::new();
    let mut skipped: Vec<String> = Vec::new();
    for (k, _, v) in &root {
        match k.as_str() {
            "schema_version" => {
                if let Some(n) = v.as_i64() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        schema_version = Some(n as u32);
                    }
                }
            }
            "updated" => {
                if let Parsed::Str(s) = v {
                    updated = Some(s.clone());
                }
            }
            "packages" => {
                if let Parsed::Object(pkgs) = v {
                    for (pname, _, pval) in pkgs {
                        // @PLN78 step 1 — SKIP a package that does not parse; do not
                        // reject the index.  One index serves every client, so a single
                        // malformed entry used to take `loft install` down for everyone:
                        // a healthy `regex` beside one under-specified entry failed with
                        // "missing `url`" and NO package resolved.  A publish mistake in
                        // one entry must cost that entry, not the registry.
                        //
                        // Sound because the signature is verified over the raw document
                        // BEFORE this runs (`install::verify_or_explain`): skipping is a
                        // choice about already-authenticated data, so it admits nothing
                        // an attacker controls.
                        match parse_package(pname, pval) {
                            Ok(pkg) => {
                                packages.insert(pname.clone(), pkg);
                            }
                            Err(e) => skipped.push(e),
                        }
                    }
                }
            }
            _ => {
                // Forward-compat: silently ignore unknown top-level keys.
            }
        }
    }
    let schema_version =
        schema_version.ok_or_else(|| "registry.json: missing `schema_version`".to_string())?;
    if schema_version != 1 {
        return Err(format!(
            "registry.json: schema_version {schema_version} unsupported — upgrade loft"
        ));
    }
    // Loud, not silent: a skipped package is a publishing bug, and the reader who
    // can act on it is whoever notices their package missing.  Summarised in one
    // line so a broken index cannot spam every command.
    if !skipped.is_empty() {
        eprintln!(
            "loft: registry index — {} package(s) skipped as unreadable: {}",
            skipped.len(),
            skipped.join("; ")
        );
    }
    Ok(RegistryIndex {
        schema_version,
        updated: updated.unwrap_or_default(),
        packages,
        skipped,
    })
}

fn parse_package(name: &str, val: &Parsed) -> Result<Package, String> {
    let Parsed::Object(fields) = val else {
        return Err(format!("package `{name}`: expected object"));
    };
    let mut pkg = Package {
        name: name.to_string(),
        description: None,
        homepage: None,
        categories: Vec::new(),
        yanked: Vec::new(),
        versions: BTreeMap::new(),
    };
    for (k, _, v) in fields {
        match k.as_str() {
            "description" => {
                if let Parsed::Str(s) = v {
                    pkg.description = Some(s.clone());
                }
            }
            "homepage" => {
                if let Parsed::Str(s) = v {
                    pkg.homepage = Some(s.clone());
                }
            }
            "categories" => {
                if let Parsed::Array(arr) = v {
                    pkg.categories = arr
                        .iter()
                        .filter_map(|p| match p {
                            Parsed::Str(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                }
            }
            "yanked" => {
                if let Parsed::Array(arr) = v {
                    pkg.yanked = arr
                        .iter()
                        .filter_map(|p| match p {
                            Parsed::Str(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                }
            }
            "versions" => {
                if let Parsed::Object(vmap) = v {
                    for (semver, _, vval) in vmap {
                        let ver = parse_version(name, semver, vval)?;
                        pkg.versions.insert(semver.clone(), ver);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(pkg)
}

fn parse_version(pkg_name: &str, semver: &str, val: &Parsed) -> Result<Version, String> {
    let Parsed::Object(fields) = val else {
        return Err(format!(
            "package `{pkg_name}` version `{semver}`: expected object"
        ));
    };
    let mut url: Option<String> = None;
    let mut sha256: Option<String> = None;
    let mut size: Option<u64> = None;
    let mut loft: Option<String> = None;
    let mut api_compatible_with: Option<String> = None;
    let mut data_compatible_with: Option<String> = None;
    let mut deps: BTreeMap<String, String> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    let mut replaces: Vec<String> = Vec::new();
    let mut provides: Vec<String> = Vec::new();
    let mut triggers: Vec<String> = Vec::new();
    let mut binaries: BTreeMap<String, BinaryEntry> = BTreeMap::new();
    let mut api: Vec<ApiItem> = Vec::new();
    let mut prerelease = false;
    let mut published: Option<String> = None;
    for (k, _, v) in fields {
        match (k.as_str(), v) {
            ("url", Parsed::Str(s)) => url = Some(s.clone()),
            ("sha256", Parsed::Str(s)) => sha256 = Some(s.clone()),
            ("size", num) => {
                // @PLN109 — accept an integer-shaped size (`Parsed::Int`) or a
                // legacy `Number`.
                if let Some(n) = num.as_i64() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        size = Some(n as u64);
                    }
                }
            }
            ("loft", Parsed::Str(s)) => loft = Some(s.clone()),
            ("api_compatible_with", Parsed::Str(s)) => api_compatible_with = Some(s.clone()),
            ("data_compatible_with", Parsed::Str(s)) => data_compatible_with = Some(s.clone()),
            ("deps", Parsed::Object(dmap)) => {
                for (dname, _, dval) in dmap {
                    if let Parsed::Str(s) = dval {
                        deps.insert(dname.clone(), s.clone());
                    }
                }
            }
            ("conflicts", Parsed::Array(a)) => {
                conflicts = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("replaces", Parsed::Array(a)) => {
                replaces = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("provides", Parsed::Array(a)) => {
                provides = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("triggers", Parsed::Array(a)) => {
                triggers = a
                    .iter()
                    .filter_map(|p| match p {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
            }
            ("binaries", Parsed::Object(bmap)) => {
                for (triple, _, bval) in bmap {
                    if let Parsed::Object(bfields) = bval {
                        let mut burl: Option<String> = None;
                        let mut bsha: Option<String> = None;
                        let mut bfp: Option<u64> = None;
                        let mut bmanifest: Option<String> = None;
                        for (bk, _, bv) in bfields {
                            match (bk.as_str(), bv) {
                                ("url", Parsed::Str(s)) => burl = Some(s.clone()),
                                ("sha256", Parsed::Str(s)) => bsha = Some(s.clone()),
                                // @PLN21 — fp stored as a string (u64 precision).
                                ("loft_ffi_fp", Parsed::Str(s)) => bfp = s.parse().ok(),
                                ("manifest_sha256", Parsed::Str(s)) => {
                                    bmanifest = Some(s.clone());
                                }
                                _ => {}
                            }
                        }
                        if let (Some(u), Some(s)) = (burl, bsha) {
                            binaries.insert(
                                triple.clone(),
                                BinaryEntry {
                                    url: u,
                                    sha256: s,
                                    loft_ffi_fp: bfp,
                                    manifest_sha256: bmanifest,
                                },
                            );
                        }
                    }
                }
            }
            ("api", Parsed::Array(a)) => {
                for item in a {
                    if let Parsed::Object(ifields) = item {
                        let mut sig: Option<String> = None;
                        let mut doc = String::new();
                        for (ik, _, iv) in ifields {
                            match (ik.as_str(), iv) {
                                ("sig", Parsed::Str(s)) => sig = Some(s.clone()),
                                ("doc", Parsed::Str(s)) => doc.clone_from(s),
                                _ => {}
                            }
                        }
                        if let Some(sig) = sig {
                            api.push(ApiItem { sig, doc });
                        }
                    }
                }
            }
            ("prerelease", Parsed::Bool(b)) => prerelease = *b,
            ("published", Parsed::Str(s)) => published = Some(s.clone()),
            _ => {}
        }
    }
    let url =
        url.ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `url`"))?;
    let sha256 = sha256
        .ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `sha256`"))?;
    let size =
        size.ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `size`"))?;
    let loft =
        loft.ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `loft`"))?;
    let published = published
        .ok_or_else(|| format!("package `{pkg_name}` version `{semver}`: missing `published`"))?;
    Ok(Version {
        semver: semver.to_string(),
        url,
        sha256,
        size,
        loft,
        api_compatible_with,
        data_compatible_with,
        deps,
        conflicts,
        replaces,
        provides,
        triggers,
        binaries,
        api,
        prerelease,
        published,
    })
}

// ── Version resolution ────────────────────────────────────────────

/// Find the best version of `pkg` matching `constraint`.  Skips
/// yanked versions and (unless `allow_prerelease`) prereleases.
///
/// Constraint shorthand:
/// - exact:  `"0.1.0"`        — only `0.1.0`, **and a yanked version still resolves**.
/// - caret:  `"^0.1.0"`       — `>=0.1.0, <0.2.0` (cargo / npm shape).
/// - tilde:  `"~0.1.0"`       — `>=0.1.0, <0.2.0` (loosened to match
///   cargo's tilde for 0.x).
/// - range:  `">=0.1, <0.3"`  — comma-separated bounds.
/// - any:    `"*"` or empty   — any non-yanked, non-prerelease.
///
/// Picks the **highest** satisfying version.
///
/// **Yanking discourages a version; it does not withdraw one.** `PKG_REGISTRY.md` keeps a
/// yanked version listed precisely so a `loft.lock` pinned to it still resolves, and skipping
/// it on an EXACT pin broke that promise — `loft install glb@0.1.1` refused a version the
/// index plainly carries, which is the same retention failure `web 0.2.2` suffered one layer
/// down. A range or `*` still skips yanked, so nothing new ever picks one up by accident.
#[must_use]
pub fn find_best_version<'a>(
    pkg: &'a Package,
    constraint: &str,
    allow_prerelease: bool,
) -> Option<&'a Version> {
    let yanked: std::collections::HashSet<&str> = pkg.yanked.iter().map(String::as_str).collect();
    // An exact pin names one release and is what a lockfile records; anything else is the
    // resolver choosing on the consumer's behalf, where a yanked version must stay excluded.
    let exact_pin = constraint.trim().starts_with(|c: char| c.is_ascii_digit());
    let mut best: Option<&Version> = None;
    for ver in pkg.versions.values() {
        if yanked.contains(ver.semver.as_str()) && !exact_pin {
            continue;
        }
        if ver.prerelease && !allow_prerelease {
            continue;
        }
        if !satisfies(&ver.semver, constraint) {
            continue;
        }
        if best
            .is_none_or(|b| compare_semver(&ver.semver, &b.semver) == std::cmp::Ordering::Greater)
        {
            best = Some(ver);
        }
    }
    best
}

/// What resolution chose for a consumer that already holds a version, and what
/// it deliberately did not choose.
///
/// `withheld` is the reason this is a struct rather than an `Option<&Version>`:
/// a resolver that silently stops at an older release teaches the consumer that
/// no upgrade exists. The whole point of a DECLARED break is that someone gets
/// told about it.
#[derive(Debug)]
pub struct Resolution<'a> {
    /// Highest satisfying version that is safe for what the consumer holds.
    pub best: Option<&'a Version>,
    /// Higher satisfying versions passed over because they declare a break past
    /// the held version, newest first.
    pub withheld: Vec<&'a Version>,
}

/// Resolve for a consumer that already holds `held`: the highest satisfying
/// version that still declares itself a drop-in for what they have.
///
/// A candidate `R` saying `api_compatible_with = F` promises it replaces
/// anything from `F` onward, so it is safe for a consumer on `held` exactly
/// when `F <= held`. Above that, `R` has declared it will break them, and the
/// registry keeps `held`'s neighbours installable precisely so they can stay.
///
/// Three cases mean "unconstrained", and all three are deliberate:
///
/// - **`held` is `None`** (a fresh install) — there is nothing to be a drop-in
///   *for*. Constraining here would resolve a first-time user to an ancient
///   release because the library broke compatibility three versions ago, which
///   is backwards: they have no old call sites to protect.
/// - **the candidate declares no floor** — it has promised nothing, so nothing
///   is enforced. This is what keeps the change inert for every version
///   published before the contract existed.
/// - **`constraint` is an exact pin** — it names ONE release, so the caller has
///   already chosen. Filtering it would report "no version satisfies the
///   constraint" for a version that plainly exists, which reads as a broken
///   registry rather than the deliberate step across a break that it is.
#[must_use]
pub fn find_compatible_version<'a>(
    pkg: &'a Package,
    constraint: &str,
    allow_prerelease: bool,
    held: Option<&str>,
) -> Resolution<'a> {
    let exact_pin = constraint.trim().starts_with(|c: char| c.is_ascii_digit());
    let Some(held) = held.filter(|_| !exact_pin) else {
        return Resolution {
            best: find_best_version(pkg, constraint, allow_prerelease),
            withheld: Vec::new(),
        };
    };
    let yanked: std::collections::HashSet<&str> = pkg.yanked.iter().map(String::as_str).collect();
    let mut safe: Option<&Version> = None;
    let mut withheld: Vec<&Version> = Vec::new();
    for ver in pkg.versions.values() {
        if yanked.contains(ver.semver.as_str())
            || (ver.prerelease && !allow_prerelease)
            || !satisfies(&ver.semver, constraint)
        {
            continue;
        }
        // A floor above what the consumer holds is a declared break.  Only
        // count it as withheld when it is a version they would otherwise move
        // UP to — a lower release they were never going to take is not news.
        let breaks = ver
            .api_compatible_with
            .as_deref()
            .is_some_and(|floor| compare_semver(floor, held) == std::cmp::Ordering::Greater);
        if breaks {
            if compare_semver(&ver.semver, held) == std::cmp::Ordering::Greater {
                withheld.push(ver);
            }
            continue;
        }
        if safe
            .is_none_or(|b| compare_semver(&ver.semver, &b.semver) == std::cmp::Ordering::Greater)
        {
            safe = Some(ver);
        }
    }
    withheld.sort_by(|a, b| compare_semver(&b.semver, &a.semver));
    Resolution {
        best: safe,
        withheld,
    }
}

/// Check whether `version` satisfies `constraint`.
#[must_use]
pub fn satisfies(version: &str, constraint: &str) -> bool {
    let c = constraint.trim();
    if c.is_empty() || c == "*" {
        return true;
    }
    if let Some(rest) = c.strip_prefix('^') {
        let parts: Vec<u32> = rest.split('.').filter_map(|p| p.parse().ok()).collect();
        if parts.len() < 2 {
            return false;
        }
        let lo = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            parts.get(2).copied().unwrap_or(0)
        );
        let hi = format!("{}.{}.0", parts[0], parts[1].saturating_add(1));
        return compare_semver(version, &lo) != std::cmp::Ordering::Less
            && compare_semver(version, &hi) == std::cmp::Ordering::Less;
    }
    if let Some(rest) = c.strip_prefix('~') {
        let parts: Vec<u32> = rest.split('.').filter_map(|p| p.parse().ok()).collect();
        if parts.len() < 2 {
            return false;
        }
        let lo = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            parts.get(2).copied().unwrap_or(0)
        );
        let hi = format!("{}.{}.0", parts[0], parts[1].saturating_add(1));
        return compare_semver(version, &lo) != std::cmp::Ordering::Less
            && compare_semver(version, &hi) == std::cmp::Ordering::Less;
    }
    if c.contains(',') {
        return c.split(',').all(|p| satisfies(version, p.trim()));
    }
    if let Some(rest) = c.strip_prefix(">=") {
        return compare_semver(version, rest.trim()) != std::cmp::Ordering::Less;
    }
    if let Some(rest) = c.strip_prefix('>') {
        return compare_semver(version, rest.trim()) == std::cmp::Ordering::Greater;
    }
    if let Some(rest) = c.strip_prefix("<=") {
        return compare_semver(version, rest.trim()) != std::cmp::Ordering::Greater;
    }
    if let Some(rest) = c.strip_prefix('<') {
        return compare_semver(version, rest.trim()) == std::cmp::Ordering::Less;
    }
    if let Some(rest) = c.strip_prefix('=') {
        return version == rest.trim();
    }
    // Bare version → exact match.
    version == c
}

/// Compare two semver strings.  Treats unparseable parts as `0`.
#[must_use]
pub fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut parts = s.splitn(3, '.');
        let major = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let patch = parts
            .next()
            .map(|x| x.split('-').next().unwrap_or(x))
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        (major, minor, patch)
    };
    parse(a).cmp(&parse(b))
}

// ── Cache + URL helpers ───────────────────────────────────────────

/// Default registry URL.  Overridden by `LOFT_REGISTRY_URL` env var.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/loft-lang/registry/main/index.json";

/// Resolve the registry URL from env or default.
#[must_use]
pub fn registry_url() -> String {
    std::env::var("LOFT_REGISTRY_URL").unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string())
}

/// Published packages exporting a FREE function named `name` — the data behind
/// the "you probably meant `pkg::name`" diagnostic (@PLN13 phase 6, the
/// diagnostics slice).
///
/// Reads the CACHED index only (`~/.loft/registry/index.json`) and never the
/// network: this runs while the compiler is already reporting an error, so it
/// must not stall, and a machine with no cache simply gets no hint.  The index
/// is signature-verified when it is FETCHED; here it is only a source of names
/// for a message, so an unverifiable or malformed cache degrades to silence
/// rather than an error.  Package names are still filtered to the registry's own
/// identifier shape, so nothing from that file can inject text into a
/// diagnostic.
///
/// Methods are deliberately excluded — a `pub fn f(self: T)` is not callable
/// bare, and a published method already resolves without a `use` via the
/// lazy-load triggers (`derive_triggers`).  Only the free-function shape the
/// bare call could actually have meant is offered.
#[must_use]
pub fn packages_exporting_fn(name: &str) -> Vec<String> {
    if name.is_empty() {
        return Vec::new();
    }
    let (index_path, _, _) = index_paths();
    let Ok(content) = std::fs::read_to_string(index_path) else {
        return Vec::new();
    };
    let Ok(index) = parse_index(&content) else {
        return Vec::new();
    };
    let mut hits: Vec<String> = index
        .packages
        .iter()
        .filter(|(pkg, _)| {
            !pkg.is_empty()
                && pkg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        })
        .filter(|(_, p)| {
            // Newest version wins: an old pin may still list a function the
            // package has since dropped, and suggesting that would send the
            // reader to an API they cannot install today.
            p.versions
                .values()
                .next_back()
                .is_some_and(|v| v.api.iter().any(|item| exports_free_fn(&item.sig, name)))
        })
        .map(|(pkg, _)| pkg.clone())
        .collect();
    hits.sort();
    hits.dedup();
    hits
}

/// Does the API signature `sig` declare a free function called `name`?
///
/// Matches `pub fn <name>(` and then rejects a `self` / `both` receiver, which
/// makes it a method rather than something a bare call could resolve to.
fn exports_free_fn(sig: &str, name: &str) -> bool {
    let Some(rest) = sig.trim_start().strip_prefix("pub fn ") else {
        return false;
    };
    let Some(args) = rest.strip_prefix(name) else {
        return false;
    };
    let Some(args) = args.strip_prefix('(') else {
        return false;
    };
    let first = args.split(',').next().unwrap_or("").trim();
    let receiver = first.split(':').next().unwrap_or("").trim();
    receiver != "self" && receiver != "both"
}

/// Local cache root (`~/.loft/registry/`).
#[must_use]
pub fn cache_dir() -> PathBuf {
    // @P332: resolve the home base from `LOFT_HOME` FIRST, falling back to
    // `dirs::home_dir()`.  `dirs::home_dir()` reads `$HOME` on Unix but
    // `USERPROFILE` / the FOLDERID_Profile known folder on Windows — it does
    // NOT honour `$HOME` there — so tests that isolate the registry by setting
    // `HOME=<tmpdir>` leaked into the REAL user profile on Windows, and
    // cross-run caching then routed every install to `skipped_cached`
    // (`install_one` saw a stale `loft.toml`), yielding `installed.len()==0`.
    // `LOFT_HOME` is honoured identically on every platform; production leaves
    // it unset, so behaviour there is unchanged.
    let home = std::env::var_os("LOFT_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".loft").join("registry")
}

/// Paths for the cached index + signature + fetched-at timestamp.
#[must_use]
pub fn index_paths() -> (PathBuf, PathBuf, PathBuf) {
    let dir = cache_dir();
    (
        dir.join("index.json"),
        dir.join("index.json.sig"),
        dir.join("index.json.fetched_at"),
    )
}

/// Replace `path` with `bytes` in one step, so a concurrent reader sees either
/// the whole old file or the whole new one.
///
/// `std::fs::write` truncates and then fills, which leaves the file SHORT for as
/// long as the write takes — on the 692 KB registry index that window is wide
/// enough for another process to read a half-file and report it as a parse
/// error.  Writing beside the target and renaming closes it: a rename is atomic
/// on both Unix and Windows, and replaces an existing destination on both.
///
/// The temporary carries the pid and a counter, so two writers — or two threads
/// of one — never collide on the same scratch name.
///
/// # Errors
/// Returns the underlying I/O error if the temporary cannot be written or the
/// rename fails.  A failed rename removes the temporary rather than leaving it
/// beside the real file.
pub fn replace_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(".tmp{}-{n}", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Cache an artifact and its detached signature together.
///
/// Each file lands atomically, so neither can be read half-written.  The pair is
/// still two renames, so a reader can in principle catch the new content against
/// the old signature; [`read_signed_pair_settling`] is the other half of that
/// story and closes it.  An empty `signature` leaves the cached one alone — that
/// is the unsigned/bundle path, where the caller has already decided the absent
/// signature is acceptable.
///
/// # Errors
/// Returns the underlying I/O error if the parent directory cannot be created or
/// either file cannot be replaced.
pub fn write_signed_pair(
    content_path: &std::path::Path,
    sig_path: &std::path::Path,
    content: &[u8],
    signature: &[u8],
) -> std::io::Result<()> {
    if let Some(parent) = content_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    replace_atomically(content_path, content)?;
    if !signature.is_empty() {
        replace_atomically(sig_path, signature)?;
    }
    Ok(())
}

/// How many times [`read_signed_pair_settling`] reads the pair before believing
/// a rejection, and how long it waits between attempts.
const SETTLE_ATTEMPTS: u32 = 5;
const SETTLE_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);

/// Read an artifact and its detached signature as a MATCHED pair, waiting out a
/// writer that is part-way through replacing them.
///
/// Two files cannot be swapped in one step on any platform we target, so a
/// reader crossing a refresh can pick up the new content beside the old
/// signature.  Verification then fails, and for the registry index that failure
/// is deliberately un-bypassable — it is the sentence a TAMPERED index produces,
/// which sends the reader looking at trust roots instead of at concurrency
/// (loft#1045).
///
/// `accept` is the oracle, and it is an exact one: a signature that verifies over
/// the content it was read with proves the two came from the same generation.  So
/// a pair this function accepts is genuinely matched, never merely plausible.
///
/// A rejection is only believed after the whole settle budget, and deliberately
/// NOT the moment the bytes stop changing: a writer sitting between its two
/// renames has published the new content and not yet the new signature, so the
/// pair is torn while both files are perfectly still.  An early-out on stillness
/// returns the torn verdict at the first re-read, which is the bug rather than a
/// saving.  The cost is paid only where the pair is rejected — a path that ends
/// in an aborted command — and it is bounded by `SETTLE_ATTEMPTS`.
///
/// Returns the last pair read whether or not it was accepted, leaving the caller
/// to phrase the failure; `accepted` says which happened.
///
/// # Errors
/// Returns the underlying I/O error if the content file cannot be read.  A missing
/// or unreadable SIGNATURE is not an error here — it reads as empty and becomes the
/// caller's verdict to pass or refuse, which is where that policy already lives.
pub fn read_signed_pair_settling(
    content_path: &std::path::Path,
    sig_path: &std::path::Path,
    accept: &mut dyn FnMut(&[u8], &[u8]) -> bool,
) -> std::io::Result<SettledPair> {
    for attempt in 0..SETTLE_ATTEMPTS {
        let content = std::fs::read(content_path)?;
        let signature = std::fs::read(sig_path).unwrap_or_default();
        if accept(&content, &signature) {
            return Ok(SettledPair {
                content,
                signature,
                accepted: true,
            });
        }
        if attempt + 1 == SETTLE_ATTEMPTS {
            return Ok(SettledPair {
                content,
                signature,
                accepted: false,
            });
        }
        std::thread::sleep(SETTLE_PAUSE);
    }
    unreachable!("the loop returns on its last attempt")
}

/// What [`read_signed_pair_settling`] saw: the pair it read last, and whether the
/// caller's oracle accepted it.
pub struct SettledPair {
    pub content: Vec<u8>,
    pub signature: Vec<u8>,
    pub accepted: bool,
}

/// Directory where a given `<pkg>-<version>` tarball extracts to.
#[must_use]
pub fn extract_dir(pkg: &str, version: &str) -> PathBuf {
    cache_dir().join(format!("{pkg}-{version}"))
}

/// Every installed package in the cache: `(name, version, dir)`, sorted by
/// name then version.  Inverts the `extract_dir` naming: a dir splits at the
/// last `-<digit>` boundary, so package names may contain dashes but a
/// version always starts with a digit.
#[must_use]
pub fn installed_packages() -> Vec<(String, String, PathBuf)> {
    let mut entries = Vec::new();
    let Ok(read) = std::fs::read_dir(cache_dir()) else {
        return entries;
    };
    for ent in read.filter_map(Result::ok) {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let Some(dirname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let bytes = dirname.as_bytes();
        let split = (1..bytes.len()).find(|&i| bytes[i - 1] == b'-' && bytes[i].is_ascii_digit());
        let Some(at) = split else { continue };
        let (name, rest) = dirname.split_at(at - 1);
        let version = rest.trim_start_matches('-').to_string();
        if !name.is_empty() && !version.is_empty() {
            entries.push((name.to_string(), version, path));
        }
    }
    entries.sort();
    entries
}

/// The newest version of `pkg` already extracted in the cache that THIS loft can load —
/// @PLN143 arc C1.
///
/// The fallback for a bare script when the registry cannot be reached: a directory
/// holding five extracted copies of a package answering "not found" is the least true
/// message available. Nothing here writes, fetches, or decides anything durable — the
/// cache is written by RUNNING, and it may only make a resolution faster, never
/// different, so reading it cannot pin a later run.
///
/// Filters, so the answer is a version that will actually load:
///
/// - **A prerelease is skipped.** `find_best_version` skips it too unless asked for it,
///   and a fallback is not the place to opt someone in.
/// - **A package whose `loft` requirement this build does not satisfy is skipped**, and
///   so is one demanding a newer contract — checked with `manifest::check_version` /
///   `check_contract`, the same functions the loader uses, so the filter cannot drift
///   from what the loader accepts. Without this the newest cached copy is picked and the
///   load then fails FATALLY, hiding the older copy that would have worked.
/// - A directory with no readable `loft.toml` is skipped: a half-extracted tarball is
///   not an installed package.
///
/// Returns `(version, entry_dir)`.
#[must_use]
pub fn newest_cached_loadable(pkg: &str) -> Option<(String, PathBuf)> {
    newest_cached_loadable_satisfying(pkg, &[])
}

/// [`newest_cached_loadable`] under declared constraints (loft#1687): the newest cached copy
/// this loft can load whose version [`satisfies`] every one of `constraints` — the range a
/// manifest declares, or a lock's exact pin.  A declared scope falls back to the cache only
/// through this, so the fallback can never answer past the declaration that governs it.
#[must_use]
pub fn newest_cached_loadable_satisfying(
    pkg: &str,
    constraints: &[String],
) -> Option<(String, PathBuf)> {
    let current = env!("CARGO_PKG_VERSION");
    let mut best: Option<(String, PathBuf)> = None;
    for (name, version, dir) in installed_packages() {
        if name != pkg
            || version.contains('-')
            || !constraints.iter().all(|c| satisfies(&version, c))
        {
            continue;
        }
        let Some(m) = crate::manifest::read_manifest(&dir.join("loft.toml").to_string_lossy())
        else {
            continue;
        };
        if m.loft_version.as_ref().is_some_and(|req| {
            crate::manifest::check_version(req, current) != crate::manifest::VersionCheck::Satisfied
        }) {
            continue;
        }
        if let Some(ref creq) = m.contract {
            match crate::manifest::check_contract(creq, crate::manifest::CONTRACT_VERSION) {
                crate::manifest::ContractCheck::Ok
                | crate::manifest::ContractCheck::Drifted { .. } => {}
                _ => continue,
            }
        }
        if best
            .as_ref()
            .is_none_or(|(b, _)| compare_semver(&version, b) == std::cmp::Ordering::Greater)
        {
            best = Some((version, dir));
        }
    }
    best
}

/// Every release of `pkg` extracted in the cache, oldest first — what a declared scope's
/// "nothing cached satisfies it" message names (loft#1687).
#[must_use]
pub fn cached_versions(pkg: &str) -> Vec<String> {
    let mut out: Vec<String> = installed_packages()
        .into_iter()
        .filter(|(name, _, _)| name == pkg)
        .map(|(_, version, _)| version)
        .collect();
    out.sort_by(|a, b| compare_semver(a, b));
    out.dedup();
    out
}

/// Build the Tier-1 lazy-load `method -> package` map from a parsed catalog.
///
/// Each version's `triggers` are `"name:receiver"` strings; the map keys on the
/// method `name` (the receiver is irrelevant to *which package* to load).  This
/// is the catalog-wide fallback consulted when a consumer calls `obj.method()`
/// against a package it has NOT declared as a dependency — the resolver looks
/// the method up here, gets the package name, and hands it to the normal
/// `lib_path` resolution (lockfile → installed → auto-install).
///
/// **Ambiguity policy:** a method provided by exactly one package maps to it; a
/// method provided by two or more *distinct* packages is OMITTED — auto-loading
/// would have to guess, so the consumer must disambiguate with an explicit
/// `use`.  Versions of the SAME package collapse to one provider, so a
/// multi-version package is never self-ambiguous.  This omit is a safety net:
/// the registry CI rejects a submission whose full `method:receiver` trigger
/// collides with another package (`validate.py` gate 4, mirrored locally by the
/// [`trigger_owners`] publish pre-check), so a true `text.matches` vs
/// `text.matches` clash never reaches the catalog.  The net still fires for the
/// rarer name-only collision (`matches:text` vs `matches:Foo`), which the
/// receiver-blind pre-scan cannot tell apart.
#[must_use]
pub fn trigger_providers(index: &RegistryIndex) -> BTreeMap<String, String> {
    let mut providers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (pname, pkg) in &index.packages {
        for ver in pkg.versions.values() {
            for trig in &ver.triggers {
                let method = trig.split(':').next().unwrap_or(trig);
                if method.is_empty() {
                    continue;
                }
                providers
                    .entry(method.to_string())
                    .or_default()
                    .insert(pname.clone());
            }
        }
    }
    providers
        .into_iter()
        .filter_map(|(method, pkgs)| {
            // Unique provider → map it; ambiguous → omit (require explicit `use`).
            (pkgs.len() == 1)
                .then(|| pkgs.into_iter().next().map(|p| (method, p)))
                .flatten()
        })
        .collect()
}

// ── Derived trigger sidecar ───────────────────────────────────────

/// Path of the derived trigger sidecar, beside the cached index.
#[must_use]
pub fn triggers_sidecar_path() -> PathBuf {
    cache_dir().join("triggers.json")
}

/// `(byte length, mtime seconds)` of the cached index — the stamp a sidecar
/// carries so a stale one is recognised from the index's METADATA, without
/// reading the index itself.  Both halves are kept because either alone can
/// miss: two writes inside one second share an mtime, and a replacement of the
/// same byte count shares a length.
fn index_stamp(index: &std::path::Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(index).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((meta.len(), mtime))
}

/// The sidecar's map, or `None` when it is absent, unreadable, or stamped for a
/// different index.  Derived data is rebuilt on any doubt, never trusted.
fn read_trigger_sidecar_at(
    sidecar: &std::path::Path,
    index: &std::path::Path,
) -> Option<BTreeMap<String, String>> {
    let (len, mtime) = index_stamp(index)?;
    let text = std::fs::read_to_string(sidecar).ok()?;
    let Parsed::Object(root) = parse_json(&text).ok()? else {
        return None;
    };
    let mut stamped_len: Option<i64> = None;
    let mut stamped_mtime: Option<i64> = None;
    let mut map = BTreeMap::new();
    for (k, _, v) in &root {
        match k.as_str() {
            "index_len" => stamped_len = v.as_i64(),
            "index_mtime" => stamped_mtime = v.as_i64(),
            "triggers" => {
                if let Parsed::Object(entries) = v {
                    for (method, _, pkg) in entries {
                        if let Parsed::Str(p) = pkg {
                            map.insert(method.clone(), p.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (stamped_len? == i64::try_from(len).ok()? && stamped_mtime? == i64::try_from(mtime).ok()?)
        .then_some(map)
}

/// Write the sidecar for a map derived from the index that carried `stamp`.
///
/// The stamp is the CALLER's, taken before the map's source was read, never
/// re-measured here: a stamp younger than the map it describes would mark stale
/// triggers as current, while one older than the map only costs a rebuild.  Of
/// the two ways to be wrong, only one is recoverable.
///
/// Best-effort: this runs while the COMPILER is asking a question, so a
/// read-only cache directory costs the fast path and nothing else.  Written to a
/// temporary name and renamed, because parallel compiles reach here at once and
/// a half-written sidecar reads as a parse failure.
fn write_trigger_sidecar_at(
    sidecar: &std::path::Path,
    stamp: (u64, u64),
    map: &BTreeMap<String, String>,
) {
    let (Ok(len), Ok(mtime)) = (i64::try_from(stamp.0), i64::try_from(stamp.1)) else {
        return;
    };
    let entries = map
        .iter()
        .map(|(method, pkg)| (method.clone(), 0, Parsed::Str(pkg.clone())))
        .collect();
    let doc = Parsed::Object(vec![
        ("index_len".to_string(), 0, Parsed::Int(len)),
        ("index_mtime".to_string(), 0, Parsed::Int(mtime)),
        ("triggers".to_string(), 0, Parsed::Object(entries)),
    ]);
    if let Some(parent) = sidecar.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = sidecar.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, crate::json::to_json_string(&doc)).is_ok()
        && std::fs::rename(&tmp, sidecar).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// [`catalog_trigger_map`] against explicit paths.
fn catalog_trigger_map_at(
    sidecar: &std::path::Path,
    index: &std::path::Path,
) -> BTreeMap<String, String> {
    if let Some(map) = read_trigger_sidecar_at(sidecar, index) {
        return map;
    }
    let (Some(stamp), Ok(content)) = (index_stamp(index), std::fs::read_to_string(index)) else {
        return BTreeMap::new();
    };
    let Ok(parsed) = parse_index(&content) else {
        return BTreeMap::new();
    };
    let map = trigger_providers(&parsed);
    write_trigger_sidecar_at(sidecar, stamp, &map);
    map
}

/// The catalog trigger map (`method -> package`) for the cached index — what the
/// parser consults when a bare `line.matches(p)` names no declared dependency.
///
/// The answer is a few hundred bytes of a 700 kB index, and the compiler asks for
/// it mid-parse, so it must not cost a full index parse: it is kept in a derived
/// sidecar (`~/.loft/registry/triggers.json`) stamped with the index's length +
/// mtime, and only a missing or stale stamp pays the parse — which writes the
/// sidecar back, so that cost lands once per index rather than once per compile.
/// Measured on a four-line program against a catalog 100× today's size:
/// `loft --check` 1.05 s → 0.03 s, peak RSS 345 MB → 12 MB.
#[must_use]
pub fn catalog_trigger_map() -> BTreeMap<String, String> {
    let (index, _, _) = index_paths();
    catalog_trigger_map_at(&triggers_sidecar_path(), &index)
}

/// Bring the derived trigger sidecar in line with an index that has just been
/// loaded.  Costs one `stat` plus a short read when it is already current, and
/// reuses the caller's parsed index when it is not — so the registry commands
/// that fetch the index are the ones that pay to rebuild it, not the next
/// compile.
///
/// `source_len` is the byte length `index` was parsed from.  A file on disk of
/// some other length is a different index — another process refreshed it between
/// that parse and this call — and stamping those bytes for THIS map would claim
/// more than the caller knows, so the sidecar is left for the next reader to
/// rebuild.
pub fn refresh_trigger_sidecar(index: &RegistryIndex, source_len: u64) {
    let (idx_path, _, _) = index_paths();
    let sidecar = triggers_sidecar_path();
    if read_trigger_sidecar_at(&sidecar, &idx_path).is_some() {
        return;
    }
    let Some(stamp) = index_stamp(&idx_path) else {
        return;
    };
    if stamp.0 != source_len {
        return;
    }
    write_trigger_sidecar_at(&sidecar, stamp, &trigger_providers(index));
}

/// Map every `method:receiver` trigger in the catalog to its owning package.
///
/// A `text.matches` trigger may be owned by exactly one package across the whole
/// registry — a consumer auto-loads on the bare `.matches()` call and the
/// language can hold only one `text.matches` method.  That uniqueness is a hard
/// gate at submission (`validate.py` gate 4); this map is the *author-side*
/// pre-check: `loft publish` looks each trigger it is about to claim up here and
/// warns when another package already owns it, so the author learns before the
/// PR is opened rather than after CI rejects it.  The registry guarantees one
/// owner per trigger; should a corrupt catalog ever carry two, the
/// alphabetically-first package wins (deterministic `BTreeMap` order).
#[must_use]
pub fn trigger_owners(index: &RegistryIndex) -> BTreeMap<String, String> {
    let mut owner: BTreeMap<String, String> = BTreeMap::new();
    for (pname, pkg) in &index.packages {
        for ver in pkg.versions.values() {
            for trig in &ver.triggers {
                if !trig.is_empty() {
                    owner.entry(trig.clone()).or_insert_with(|| pname.clone());
                }
            }
        }
    }
    owner
}

// ── HTTPS fetcher ─────────────────────────────────────────────────

/// Result of fetching the registry index.
pub struct FetchedIndex {
    pub content: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Fetch `index.json` + `index.json.sig` from the given URL prefix.
///
/// The URL passed in points at the `index.json` itself; the signature
/// is fetched from the same URL with `.sig` appended.
///
/// # Errors
///
/// Returns a `String` error on HTTP failure, non-200 status, or
/// missing signature file (the latter is only an error when
/// signature verification is required; the caller decides).
pub fn fetch_index(url: &str) -> Result<FetchedIndex, String> {
    let content = http_get_bytes(url).map_err(|e| format!("fetching {url}: {e}"))?;
    let sig_url = format!("{url}.sig");
    let signature = http_get_bytes(&sig_url).unwrap_or_default();
    Ok(FetchedIndex { content, signature })
}

/// Download `url` to the local path `dest`, returning the bytes
/// fetched so the caller can hash them.
///
/// # Errors
///
/// Returns a `String` error on HTTP / IO failure.
pub fn download_tarball(url: &str, dest: &std::path::Path) -> Result<Vec<u8>, String> {
    let bytes = fetch_bytes(url)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    // Atomically: a prebuilt cdylib lands here, and another process may load it the moment
    // the name exists — a plain write would hand that process a short file.
    replace_atomically(dest, &bytes).map_err(|e| format!("writing {}: {e}", dest.display()))?;
    Ok(bytes)
}

/// Fetch `url` in full, with the download's error spelling.
///
/// # Errors
/// The transport's error, prefixed with the URL.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    http_get_bytes(url).map_err(|e| format!("downloading {url}: {e}"))
}

/// Default ceiling on a single HTTP response — the registry index, or a package
/// tarball.  A ceiling has to exist (an unbounded response is an unbounded
/// allocation), but it must never be reached by SILENCE: the previous 50 MB cap
/// TRUNCATED the body and handed the caller a short file, which surfaced as
/// `JSON parse error: unterminated string` at byte 52,428,800 rather than as
/// "the index is bigger than this loft will read".  That number is compiled into
/// every released binary, so an index growing past it would have taken the
/// registry down for every client already in the wild, with nothing the registry
/// itself could do about it.  Raised, and — the part that matters — turned into a
/// refusal, so the day it is reached the message names the ceiling and the way
/// past it.
pub const DEFAULT_MAX_DOWNLOAD: u64 = 512 << 20;

/// The download ceiling in bytes, or `0` for none — `LOFT_MAX_DOWNLOAD`
/// overriding [`DEFAULT_MAX_DOWNLOAD`], written the way a person writes a size
/// (`1G`, `512M`).  An unparseable value keeps the default and says so: a typo
/// in a limit must not silently remove the limit.
#[must_use]
pub fn max_download_bytes() -> u64 {
    let Ok(v) = std::env::var("LOFT_MAX_DOWNLOAD") else {
        return DEFAULT_MAX_DOWNLOAD;
    };
    if let Some(bytes) = crate::store_budget::parse_limit(&v) {
        bytes
    } else {
        eprintln!(
            "loft: LOFT_MAX_DOWNLOAD='{v}' is not a size (try 1G, 512M or 0) — \
             keeping the default {}",
            crate::store_budget::human(DEFAULT_MAX_DOWNLOAD)
        );
        DEFAULT_MAX_DOWNLOAD
    }
}

/// Read a whole response body, REFUSING one longer than `limit` bytes (`0` = no
/// limit) rather than truncating it.  Reads one byte past the ceiling, so a body
/// exactly at it still passes and the first byte over is what trips the refusal.
/// The message leaves the URL to the caller, which already names it.
fn read_capped(reader: impl std::io::Read, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let ceiling = if limit == 0 {
        u64::MAX
    } else {
        limit.saturating_add(1)
    };
    let mut buf = Vec::new();
    reader
        .take(ceiling)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    if limit != 0 && u64::try_from(buf.len()).unwrap_or(u64::MAX) > limit {
        return Err(format!(
            "response is larger than the {} download ceiling — refusing it rather \
             than handing back a truncated file; raise the ceiling with \
             LOFT_MAX_DOWNLOAD=<size> (e.g. 1G, or 0 for no ceiling)",
            crate::store_budget::human(limit)
        ));
    }
    Ok(buf)
}

/// Fetch `url` in full.  A `file://` URL reads the local path directly and is not
/// subject to the download ceiling — nothing about a local mirror is unbounded in
/// the way a remote response is.
pub(crate) fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    // @PLAN12 Phase 6.11 — support `file://` URLs for offline
    // mirrors + bundle-import-served indexes.  Same contract as the
    // HTTP path: return the raw bytes at the URL.
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| format!("file:// read error for {path}: {e}"));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_mins(1))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|e| format!("HTTP error: {e}"))?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} for {url}"));
    }
    read_capped(response.into_reader(), max_download_bytes())
}

// ── Tarball extraction ────────────────────────────────────────────

/// Extract a gzipped tarball from `tarball_path` into `dest_parent`.
/// The tarball's top-level directory (e.g. `<pkg>-<version>/`) becomes
/// a subdirectory of `dest_parent`.
///
/// # Errors
///
/// IO errors propagate as `String`.  Missing parent dir is created
/// implicitly.
pub fn extract_tarball(
    tarball_path: &std::path::Path,
    dest_parent: &std::path::Path,
) -> Result<(), String> {
    std::fs::create_dir_all(dest_parent)
        .map_err(|e| format!("create {}: {e}", dest_parent.display()))?;
    let f = std::fs::File::open(tarball_path)
        .map_err(|e| format!("open {}: {e}", tarball_path.display()))?;
    let dec = flate2::read::GzDecoder::new(f);
    let mut ar = tar::Archive::new(dec);
    ar.unpack(dest_parent)
        .map_err(|e| format!("extract {}: {e}", tarball_path.display()))?;
    Ok(())
}

/// Unpack a gzipped tarball held in memory into `dest_parent`, as [`extract_tarball`] does
/// for one on disk.
///
/// # Errors
/// IO errors propagate as `String`.
pub fn unpack_tarball_bytes(bytes: &[u8], dest_parent: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dest_parent)
        .map_err(|e| format!("create {}: {e}", dest_parent.display()))?;
    let dec = flate2::read::GzDecoder::new(bytes);
    tar::Archive::new(dec)
        .unpack(dest_parent)
        .map_err(|e| format!("extract into {}: {e}", dest_parent.display()))
}

/// Place a verified package tarball at [`extract_dir`]`(pkg, version)` in ONE step, so the
/// directory other processes test for (`loft.toml` inside it) never exists half-filled.
///
/// Several processes install the same package at once — a server and its clients each
/// resolving `use web`, or one test process per test — and every one of them used to unpack
/// into the shared directory from a shared scratch tarball: one install deleted the tarball
/// another was about to open, rewrote the one another was reading, or truncated a source file
/// another had already found and was parsing.  Each install now unpacks into a staging
/// directory of its own and renames the package directory into place.  The first rename
/// wins; a later one finds the directory complete and answers `false`, and that install
/// counts the package as already cached.
///
/// A tarball whose top-level directory is not `<pkg>-<version>/`, and a directory left
/// without its `loft.toml` by an interrupted older install, are unpacked straight into the
/// cache as before: the rename has nothing it can promise for either.
///
/// # Errors
/// IO errors propagate as `String`.
pub fn place_package(bytes: &[u8], pkg: &str, version: &str) -> Result<bool, String> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let cache = cache_dir();
    let dest = extract_dir(pkg, version);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = cache.join(format!(
        ".staging-{pkg}-{version}-{}-{n}",
        std::process::id()
    ));
    let placed = unpack_tarball_bytes(bytes, &staging).and_then(|()| {
        let top = staging.join(format!("{pkg}-{version}"));
        if !top.join("loft.toml").exists() {
            return unpack_tarball_bytes(bytes, &cache).map(|()| true);
        }
        match std::fs::rename(&top, &dest) {
            Ok(()) => Ok(true),
            Err(_) if dest.join("loft.toml").exists() => Ok(false),
            Err(_) => unpack_tarball_bytes(bytes, &cache).map(|()| true),
        }
    });
    let _ = std::fs::remove_dir_all(&staging);
    placed
}

/// Render the registry catalog: one line per package — name, latest STABLE
/// version (yanked + prereleases skipped, via `find_best_version`), and the
/// one-line description.  Packages come out alphabetical (`packages` is a
/// `BTreeMap`).  Backs `loft api --registry` (printed) and the
/// `.loft/api/_available.api` discovery file, so an agent sees what it can
/// `loft install`, not just what's already installed.
#[must_use]
pub fn render_catalog(index: &RegistryIndex) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# loft registry — {} available packages",
        index.packages.len()
    );
    let _ = writeln!(
        out,
        "# install: `loft install <name>`   ·   surface: `loft api <name>`"
    );
    let _ = writeln!(out);
    for (name, pkg) in &index.packages {
        let latest = find_best_version(pkg, "*", false).map_or("?", |v| v.semver.as_str());
        match pkg.description.as_deref() {
            Some(d) if !d.is_empty() => {
                let _ = writeln!(out, "{name} {latest} — {d}");
            }
            _ => {
                let _ = writeln!(out, "{name} {latest}");
            }
        }
    }
    out
}

/// Rank the packages matching `query` for `loft search`.  `query` is matched
/// case-insensitively (lowercase it before calling) against the package name,
/// description, and categories; results are ordered **exact-name → name-prefix
/// → name/description/category substring**, alphabetical within each tier.  An
/// empty query returns every package alphabetically (the full listing).
#[must_use]
pub fn rank_hits<'a>(index: &'a RegistryIndex, query: &str) -> Vec<&'a Package> {
    let mut scored: Vec<(u8, &Package)> = Vec::new();
    for pkg in index.packages.values() {
        let name = pkg.name.to_ascii_lowercase();
        let tier = if query.is_empty() {
            3
        } else if name == query {
            0
        } else if name.starts_with(query) {
            1
        } else if name.contains(query)
            || pkg
                .description
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(query)
            || pkg
                .categories
                .iter()
                .any(|c| c.to_ascii_lowercase().contains(query))
        {
            2
        } else {
            continue;
        };
        scored.push((tier, pkg));
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().map(|(_, pkg)| pkg).collect()
}

/// One function-aware `loft search` result: a package (or the stdlib) plus the
/// functions of it whose signature or one-line doc matched the query.  `fns` is
/// empty for a metadata-only hit (the package name / description / category
/// matched, but no individual function did) or for an empty query (the full
/// package listing).  `tier` is the ranking bucket (lower = better).
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub version: String,
    pub is_stdlib: bool,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub auto_use: bool,
    pub fns: Vec<ApiItem>,
    pub tier: u8,
}

/// Google-like match: EVERY whitespace-separated term of the (already lowercased)
/// query `q` must appear somewhere in the item's signature or its full doc
/// paragraph.  So `hash hex` narrows to items mentioning both; an all-whitespace
/// query matches nothing.
fn item_matches(item: &ApiItem, q: &str) -> bool {
    let hay = format!("{}\n{}", item.sig, item.doc).to_ascii_lowercase();
    let mut saw_term = false;
    for term in q.split_whitespace() {
        saw_term = true;
        if !hay.contains(term) {
            return false;
        }
    }
    saw_term
}

/// Function-aware search (S6–S9): rank packages by metadata AND surface the
/// individual functions matching `query`, across the registry `index` and the
/// embedded `stdlib` API.  `query` must be lowercased by the caller.  Ordering:
/// exact-name → name-prefix → **has-matching-function** → description/category
/// substring; within a tier the stdlib sorts first (built in, no install), then
/// alphabetical by name.  An empty query lists every package (no functions),
/// matching the S0–S5 full listing.
#[must_use]
pub fn search_results(index: &RegistryIndex, stdlib: &[ApiItem], query: &str) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();
    // The stdlib is surfaced ONLY by a function match: it has no package name or
    // description to query, and an empty query lists registry packages.
    if !query.is_empty() {
        let fns: Vec<ApiItem> = stdlib
            .iter()
            .filter(|a| item_matches(a, query))
            .cloned()
            .collect();
        if !fns.is_empty() {
            out.push(SearchResult {
                name: "stdlib".to_string(),
                version: String::new(),
                is_stdlib: true,
                description: None,
                categories: Vec::new(),
                auto_use: false,
                fns,
                tier: 2,
            });
        }
    }
    for pkg in index.packages.values() {
        let name = pkg.name.to_ascii_lowercase();
        let latest = find_best_version(pkg, "*", false);
        let fns: Vec<ApiItem> = if query.is_empty() {
            Vec::new()
        } else {
            latest.map_or_else(Vec::new, |v| {
                v.api
                    .iter()
                    .filter(|a| item_matches(a, query))
                    .cloned()
                    .collect()
            })
        };
        let meta = !query.is_empty()
            && (name.contains(query)
                || pkg
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(query)
                || pkg
                    .categories
                    .iter()
                    .any(|c| c.to_ascii_lowercase().contains(query)));
        let tier = if query.is_empty() {
            3
        } else if name == query {
            0
        } else if name.starts_with(query) {
            1
        } else if !fns.is_empty() {
            2
        } else if meta {
            3
        } else {
            continue;
        };
        out.push(SearchResult {
            name: pkg.name.clone(),
            version: latest.map_or_else(|| "(no stable version)".to_string(), |v| v.semver.clone()),
            is_stdlib: false,
            description: pkg.description.clone(),
            categories: pkg.categories.clone(),
            auto_use: latest.is_some_and(|v| !v.triggers.is_empty()),
            fns,
            tier,
        });
    }
    out.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| b.is_stdlib.cmp(&a.is_stdlib))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── @PLN78 step 1 — one bad entry must not take the registry down ────────────
    //
    // The index is ONE document serving every client, so the blast radius of a
    // publishing mistake is the whole ecosystem unless parsing is per-package.
    // These pin both halves: the healthy packages survive, and the damage is
    // reported rather than swallowed.

    /// The shape that motivated it: a well-formed package beside one whose version
    /// omits a mandatory field.  Before, `parse_index` returned `Err` and NOTHING
    /// resolved — `loft install regex` failed because an unrelated entry was broken.
    #[test]
    fn a_malformed_package_is_skipped_not_fatal() {
        let doc = r#"{
          "schema_version": 1, "updated": "2026-07-31T00:00:00Z",
          "packages": {
            "regex": { "versions": { "0.1.0": {
                "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                "published": "2026-05-31T00:00:00Z" } } },
            "broken": { "versions": { "2026.7.2": {
                "published": "2026-07-21T00:00:00Z" } } }
          }
        }"#;
        let idx = parse_index(doc).expect("a malformed entry must not reject the index");
        assert!(
            idx.packages.contains_key("regex"),
            "the healthy package must still resolve"
        );
        assert!(
            !idx.packages.contains_key("broken"),
            "the malformed package must not be offered"
        );
        assert_eq!(
            idx.skipped.len(),
            1,
            "the damage must be reported, not swallowed"
        );
        assert!(
            idx.skipped[0].contains("broken") && idx.skipped[0].contains("url"),
            "the report must name the package and the missing field: {:?}",
            idx.skipped[0]
        );
    }

    /// The control: a clean index reports nothing skipped.  Without it, a parser
    /// that skipped EVERYTHING would also satisfy the test above.
    #[test]
    fn a_clean_index_skips_nothing() {
        let doc = r#"{
          "schema_version": 1, "updated": "2026-07-31T00:00:00Z",
          "packages": { "regex": { "versions": { "0.1.0": {
              "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
              "published": "2026-05-31T00:00:00Z" } } } }
        }"#;
        let idx = parse_index(doc).expect("parse");
        assert!(idx.skipped.is_empty(), "clean index: {:?}", idx.skipped);
        assert_eq!(idx.packages.len(), 1);
    }

    /// Tolerance is per-PACKAGE, not per-document: a structurally broken index
    /// (bad JSON, wrong schema_version) is still fatal, because then nothing in it
    /// can be trusted to mean what it says.
    #[test]
    fn a_structurally_broken_index_is_still_fatal() {
        assert!(parse_index("not json at all").is_err());
        assert!(
            parse_index(r#"{"schema_version": 99, "packages": {}}"#).is_err(),
            "an unsupported schema_version must not be parsed leniently"
        );
        assert!(
            parse_index(r#"{"packages": {}}"#).is_err(),
            "a missing schema_version must stay fatal"
        );
    }

    #[test]
    fn rank_hits_orders_exact_prefix_then_description() {
        use std::collections::BTreeMap;
        let mk = |name: &str, desc: &str| Package {
            name: name.to_string(),
            description: Some(desc.to_string()),
            homepage: None,
            categories: Vec::new(),
            yanked: Vec::new(),
            versions: BTreeMap::new(),
        };
        let mut packages = BTreeMap::new();
        packages.insert("stringy".to_string(), mk("stringy", "text manipulation")); // desc hit
        packages.insert("text".to_string(), mk("text", "string ops")); // exact
        packages.insert("text_utils".to_string(), mk("text_utils", "helpers")); // prefix
        let index = RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages,
            skipped: Vec::new(),
        };
        let ranked: Vec<&str> = rank_hits(&index, "text")
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(ranked, ["text", "text_utils", "stringy"]);
    }

    const SAMPLE: &str = r#"{
        "schema_version": 1,
        "updated": "2026-05-24T08:00:00Z",
        "packages": {
            "crypto": {
                "description": "SHA-256 etc.",
                "categories": ["crypto"],
                "yanked": ["0.1.0"],
                "versions": {
                    "0.1.0": {
                        "url": "https://example.com/crypto-0.1.0.tar.gz",
                        "sha256": "abc",
                        "size": 100,
                        "loft": ">=0.8",
                        "published": "2026-05-01T00:00:00Z"
                    },
                    "0.1.1": {
                        "url": "https://example.com/crypto-0.1.1.tar.gz",
                        "sha256": "def",
                        "size": 110,
                        "loft": ">=0.8",
                        "deps": {"hash": ">=0.1"},
                        "published": "2026-05-15T00:00:00Z"
                    },
                    "0.2.0-beta": {
                        "url": "https://example.com/crypto-0.2.0-beta.tar.gz",
                        "sha256": "ghi",
                        "size": 120,
                        "loft": ">=0.8",
                        "prerelease": true,
                        "published": "2026-05-20T00:00:00Z"
                    }
                }
            }
        }
    }"#;

    /// `render_catalog` lists each package with its latest STABLE version
    /// (yanked + prereleases skipped) + description — the agent discovery view.
    #[test]
    fn render_catalog_shows_latest_stable() {
        let index = parse_index(SAMPLE).expect("parse SAMPLE");
        let cat = render_catalog(&index);
        assert!(
            cat.contains("# loft registry — 1 available packages"),
            "header/count: {cat}"
        );
        // 0.1.0 is yanked, 0.2.0-beta is prerelease → latest stable is 0.1.1.
        assert!(
            cat.contains("crypto 0.1.1 — SHA-256 etc."),
            "latest-stable + description line: {cat}"
        );
    }

    #[test]
    fn parses_triggers_field() {
        let doc = r#"{
            "schema_version": 1,
            "updated": "2026-05-31T00:00:00Z",
            "packages": {
                "regex": {
                    "description": "regex",
                    "categories": [],
                    "yanked": [],
                    "versions": {
                        "0.1.0": {
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["matches:text", "regex_find:text"],
                            "published": "2026-05-31T00:00:00Z"
                        }
                    }
                }
            }
        }"#;
        let idx = parse_index(doc).expect("parse");
        let v = &idx.packages["regex"].versions["0.1.0"];
        assert_eq!(
            v.triggers,
            vec!["matches:text".to_string(), "regex_find:text".to_string()]
        );
    }

    #[test]
    fn trigger_providers_unique_maps_ambiguous_omits() {
        // `matches` is on text in two DISTINCT packages → receiver-blind
        // pre-scan can't choose → omitted.  `regex_find` is unique → mapped.
        // `slugify` lives in two VERSIONS of one package → collapses, mapped.
        let doc = r#"{
            "schema_version": 1,
            "updated": "2026-05-31T00:00:00Z",
            "packages": {
                "regex": {
                    "categories": [], "yanked": [],
                    "versions": {
                        "0.1.0": {
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["matches:text", "regex_find:text"],
                            "published": "2026-05-31T00:00:00Z"
                        }
                    }
                },
                "glob": {
                    "categories": [], "yanked": [],
                    "versions": {
                        "0.2.0": {
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["matches:text"],
                            "published": "2026-05-31T00:00:00Z"
                        }
                    }
                },
                "slug": {
                    "categories": [], "yanked": [],
                    "versions": {
                        "0.1.0": {
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["slugify:text"],
                            "published": "2026-05-31T00:00:00Z"
                        },
                        "0.2.0": {
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["slugify:text"],
                            "published": "2026-05-31T00:00:00Z"
                        }
                    }
                }
            }
        }"#;
        let idx = parse_index(doc).expect("parse");
        let map = trigger_providers(&idx);
        assert_eq!(map.get("regex_find"), Some(&"regex".to_string()));
        assert_eq!(map.get("slugify"), Some(&"slug".to_string()));
        assert_eq!(map.get("matches"), None, "ambiguous method must be omitted");
    }

    #[test]
    fn trigger_owners_maps_each_trigger_to_its_package() {
        // Full `method:receiver` -> package map, used by the publish pre-check.
        // The same package across two versions collapses to one owner.
        let doc = r#"{
            "schema_version": 1, "updated": "x",
            "packages": {
                "regex": { "categories": [], "yanked": [], "versions": {
                    "0.1.0": { "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                        "triggers": ["matches:text"], "published": "x" },
                    "0.2.0": { "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                        "triggers": ["matches:text", "regex_find:text"], "published": "x" } } },
                "slug": { "categories": [], "yanked": [], "versions": {
                    "0.1.0": { "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                        "triggers": ["slugify:text"], "published": "x" } } }
            }
        }"#;
        let idx = parse_index(doc).expect("parse");
        let owners = trigger_owners(&idx);
        assert_eq!(owners.get("matches:text"), Some(&"regex".to_string()));
        assert_eq!(owners.get("regex_find:text"), Some(&"regex".to_string()));
        assert_eq!(owners.get("slugify:text"), Some(&"slug".to_string()));
        assert_eq!(owners.get("nope:text"), None);
    }

    #[test]
    fn parses_sample_index() {
        let idx = parse_index(SAMPLE).expect("parse");
        assert_eq!(idx.schema_version, 1);
        let crypto = idx.packages.get("crypto").expect("crypto");
        assert_eq!(crypto.categories, vec!["crypto"]);
        assert_eq!(crypto.yanked, vec!["0.1.0"]);
        assert_eq!(crypto.versions.len(), 3);
        let v_011 = crypto.versions.get("0.1.1").expect("0.1.1");
        assert_eq!(v_011.url, "https://example.com/crypto-0.1.1.tar.gz");
        assert_eq!(v_011.size, 110);
        assert_eq!(v_011.deps.get("hash"), Some(&">=0.1".to_string()));
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let bad = r#"{"schema_version": 999, "updated": "x", "packages": {}}"#;
        assert!(parse_index(bad).is_err());
    }

    /// Four releases across one declared break, plus a package that declares
    /// nothing — the two halves step 6 has to get right at once.
    ///
    /// `lib` 0.3.0 raises `api_compatible_with` to itself, so 0.3.0 and 0.4.0
    /// are drop-ins only for consumers already on 0.3.0 or later.  `legacy`
    /// mirrors every version published before the contract existed.
    const FLOORS: &str = r#"{
        "schema_version": 1,
        "updated": "2026-07-28T08:00:00Z",
        "packages": {
            "lib": {
                "categories": [], "yanked": [],
                "versions": {
                    "0.1.0": {"url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p"},
                    "0.2.0": {"url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p",
                              "api_compatible_with":"0.1.0"},
                    "0.3.0": {"url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p",
                              "api_compatible_with":"0.3.0"},
                    "0.4.0": {"url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p",
                              "api_compatible_with":"0.3.0"}
                }
            },
            "legacy": {
                "categories": [], "yanked": [],
                "versions": {
                    "0.1.0": {"url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p"},
                    "0.4.0": {"url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p"}
                }
            }
        }
    }"#;

    /// Step 6: an upgrade stops below a declared break, and says which releases
    /// it stopped below.
    #[test]
    fn resolution_honours_declared_floors() {
        let idx = parse_index(FLOORS).expect("parse");
        let lib = idx.packages.get("lib").expect("lib");
        let resolve = |held: Option<&str>| {
            let r = find_compatible_version(lib, "*", false, held);
            (
                r.best.map(|v| v.semver.clone()).unwrap_or_default(),
                r.withheld
                    .iter()
                    .map(|v| v.semver.clone())
                    .collect::<Vec<_>>(),
            )
        };

        // A FRESH install has no old call sites to protect, so it takes the
        // newest.  Constraining here would hand a first-time user an ancient
        // release because the library broke compatibility three versions ago.
        assert_eq!(resolve(None), ("0.4.0".to_string(), vec![]));

        // Held below the break: stop at the last drop-in, and name BOTH
        // releases that were passed over, newest first.
        assert_eq!(
            resolve(Some("0.1.0")),
            (
                "0.2.0".to_string(),
                vec!["0.4.0".to_string(), "0.3.0".to_string()]
            )
        );
        assert_eq!(
            resolve(Some("0.2.0")),
            (
                "0.2.0".to_string(),
                vec!["0.4.0".to_string(), "0.3.0".to_string()]
            )
        );

        // Held AT or past the break: the newest is a declared drop-in again,
        // and nothing is withheld.
        assert_eq!(resolve(Some("0.3.0")), ("0.4.0".to_string(), vec![]));
        assert_eq!(resolve(Some("0.4.0")), ("0.4.0".to_string(), vec![]));
    }

    /// A version that declares no floor has promised nothing, so nothing is
    /// enforced.  This is what keeps step 6 inert for every version published
    /// before the contract existed — without it, landing the resolver change
    /// would silently alter what every consumer in the registry resolves to.
    #[test]
    fn resolution_is_inert_without_declared_floors() {
        let idx = parse_index(FLOORS).expect("parse");
        let legacy = idx.packages.get("legacy").expect("legacy");
        let r = find_compatible_version(legacy, "*", false, Some("0.1.0"));
        assert_eq!(r.best.map(|v| v.semver.as_str()), Some("0.4.0"));
        assert!(r.withheld.is_empty());
        // Identical to the pre-step-6 answer, which is the actual claim.
        assert_eq!(
            r.best.map(|v| &v.semver),
            find_best_version(legacy, "*", false).map(|v| &v.semver)
        );
    }

    /// An exact pin names ONE release, so the caller has already chosen.
    #[test]
    fn an_exact_pin_crosses_a_declared_break() {
        let idx = parse_index(FLOORS).expect("parse");
        let lib = idx.packages.get("lib").expect("lib");
        let r = find_compatible_version(lib, "0.4.0", false, Some("0.1.0"));
        assert_eq!(r.best.map(|v| v.semver.as_str()), Some("0.4.0"));
        assert!(r.withheld.is_empty());
    }

    /// The floors travel in the index, so a resolver reads a release's promise
    /// without downloading and unpacking its tarball.
    #[test]
    fn index_parses_both_compatibility_floors() {
        let idx = parse_index(FLOORS).expect("parse");
        let v = &idx.packages["lib"].versions["0.3.0"];
        assert_eq!(v.api_compatible_with.as_deref(), Some("0.3.0"));
        assert_eq!(
            idx.packages["lib"].versions["0.1.0"].api_compatible_with,
            None
        );
    }

    #[test]
    fn find_best_version_skips_yanked() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        // 0.1.0 is yanked; 0.1.1 is non-prerelease; 0.2.0-beta is prerelease.
        let best = find_best_version(crypto, "^0.1", false).expect("Some");
        assert_eq!(best.semver, "0.1.1");
    }

    /// A yanked version stays INSTALLABLE by exact pin — that is the whole reason
    /// `PKG_REGISTRY.md` keeps it listed. Skipping it here refused a version the index
    /// plainly carries (`loft install glb@0.1.1`), breaking every `loft.lock` pinned across a
    /// yank: the same retention promise `web 0.2.2` broke one layer down.
    #[test]
    fn an_exact_pin_still_resolves_a_yanked_version() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        assert_eq!(
            find_best_version(crypto, "0.1.0", false).map(|v| v.semver.as_str()),
            Some("0.1.0"),
            "a lockfile pin to a yanked version must still resolve"
        );
        // ...while nothing that lets the RESOLVER choose ever picks one up.
        for c in ["*", "^0.1", ">=0.1"] {
            assert_ne!(
                find_best_version(crypto, c, false).map(|v| v.semver.as_str()),
                Some("0.1.0"),
                "constraint `{c}` must not select a yanked version"
            );
        }
    }

    #[test]
    fn find_best_version_allows_prerelease() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        let best = find_best_version(crypto, "*", true).expect("Some");
        assert_eq!(best.semver, "0.2.0-beta");
    }

    #[test]
    fn find_best_version_skips_prerelease_by_default() {
        let idx = parse_index(SAMPLE).expect("parse");
        let crypto = idx.packages.get("crypto").expect("crypto");
        let best = find_best_version(crypto, "*", false).expect("Some");
        assert_eq!(best.semver, "0.1.1");
    }

    #[test]
    fn satisfies_caret() {
        assert!(satisfies("0.1.5", "^0.1.0"));
        assert!(satisfies("0.1.0", "^0.1.0"));
        assert!(!satisfies("0.2.0", "^0.1.0"));
        assert!(!satisfies("0.0.9", "^0.1.0"));
    }

    #[test]
    fn satisfies_range() {
        assert!(satisfies("0.2.5", ">=0.2, <0.3"));
        assert!(!satisfies("0.3.0", ">=0.2, <0.3"));
        assert!(!satisfies("0.1.9", ">=0.2, <0.3"));
    }

    #[test]
    fn satisfies_star_or_empty() {
        assert!(satisfies("99.99.99", "*"));
        assert!(satisfies("99.99.99", ""));
    }

    #[test]
    fn compare_semver_basics() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_semver("1.2.3", "1.2.3"), Equal);
        assert_eq!(compare_semver("1.2.3", "1.2.4"), Less);
        assert_eq!(compare_semver("1.3.0", "1.2.99"), Greater);
        assert_eq!(compare_semver("2.0.0", "1.99.99"), Greater);
    }

    #[test]
    fn cache_dir_uses_dot_loft() {
        let dir = cache_dir();
        assert!(dir.ends_with(".loft/registry"));
    }

    #[test]
    fn extract_dir_format() {
        let p = extract_dir("crypto", "0.1.0");
        assert!(p.ends_with("crypto-0.1.0"));
    }

    #[test]
    fn registry_url_respects_env_override() {
        // Don't actually set the env var (cross-test isolation), just
        // confirm the default fallback shape.
        assert_eq!(
            std::env::var("LOFT_REGISTRY_URL")
                .ok()
                .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string()),
            registry_url()
        );
    }

    fn ver_api(semver: &str, api: Vec<ApiItem>) -> Version {
        use std::collections::BTreeMap;
        Version {
            semver: semver.to_string(),
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
            api,
            prerelease: false,
            published: "p".to_string(),
        }
    }

    #[test]
    fn search_results_surfaces_functions_stdlib_and_metadata() {
        use std::collections::BTreeMap;
        let item = |sig: &str, doc: &str| ApiItem {
            sig: sig.to_string(),
            doc: doc.to_string(),
        };
        let mut versions = BTreeMap::new();
        versions.insert(
            "0.1.0".to_string(),
            ver_api(
                "0.1.0",
                vec![item(
                    "pub fn sha256(d: vector<u8>) -> text",
                    "SHA-256 digest",
                )],
            ),
        );
        let mut packages = BTreeMap::new();
        packages.insert(
            "crypto".to_string(),
            Package {
                name: "crypto".to_string(),
                description: Some("hashing".to_string()),
                homepage: None,
                categories: vec![],
                yanked: vec![],
                versions,
            },
        );
        let index = RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages,
            skipped: Vec::new(),
        };
        let stdlib = vec![item(
            "pub fn starts_with(self: text, p: text) -> boolean",
            "prefix test",
        )];

        // A function INSIDE a registry package is surfaced, grouped under it (tier 2).
        let r = search_results(&index, &stdlib, "sha256");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "crypto");
        assert!(!r[0].is_stdlib);
        assert_eq!(r[0].fns.len(), 1);
        assert_eq!(r[0].tier, 2);

        // A stdlib function is surfaced and tagged stdlib.
        let r = search_results(&index, &stdlib, "starts_with");
        assert_eq!(r.len(), 1);
        assert!(r[0].is_stdlib);
        assert_eq!(r[0].fns.len(), 1);

        // An exact package-name match outranks a function match (tier 0).
        let r = search_results(&index, &stdlib, "crypto");
        assert_eq!(r[0].name, "crypto");
        assert_eq!(r[0].tier, 0);

        // Empty query is the full package listing — no stdlib, no functions.
        let r = search_results(&index, &stdlib, "");
        assert_eq!(r.len(), 1);
        assert!(r[0].fns.is_empty());
        assert!(!r.iter().any(|x| x.is_stdlib));

        // A miss returns nothing.
        assert!(search_results(&index, &stdlib, "nonexistent_xyz").is_empty());
    }

    #[test]
    fn parse_index_reads_api_field_and_defaults_empty() {
        let with_api = r#"{"schema_version":1,"updated":"","packages":{"crypto":{"versions":{"0.1.0":{
            "url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p",
            "api":[{"sig":"pub fn sha256(d: vector<u8>) -> text","doc":"digest"}]}}}}}"#;
        let idx = parse_index(with_api).expect("parse with api");
        let v = &idx.packages["crypto"].versions["0.1.0"];
        assert_eq!(v.api.len(), 1);
        assert_eq!(v.api[0].sig, "pub fn sha256(d: vector<u8>) -> text");
        assert_eq!(v.api[0].doc, "digest");

        // An index WITHOUT `api` (older schema) defaults to empty, never errors.
        let no_api = r#"{"schema_version":1,"updated":"","packages":{"crypto":{"versions":{"0.1.0":{
            "url":"u","sha256":"s","size":1,"loft":">=0.8","published":"p"}}}}}"#;
        let idx2 = parse_index(no_api).expect("parse without api");
        assert!(idx2.packages["crypto"].versions["0.1.0"].api.is_empty());
    }

    #[test]
    fn search_matches_all_query_terms_google_like() {
        use std::collections::BTreeMap;
        let stdlib = vec![ApiItem {
            sig: "pub fn sha256(data: text) -> text".to_string(),
            doc: "SHA-256 hash of a string.\nReturns a 64-char hex string.".to_string(),
        }];
        let index = RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages: BTreeMap::new(),
            skipped: Vec::new(),
        };
        // Both terms present (one per doc line) → hit.
        assert_eq!(search_results(&index, &stdlib, "hash hex").len(), 1);
        // One term in the SIG, one in the doc → hit (the haystack is sig + doc).
        assert_eq!(search_results(&index, &stdlib, "sha256 returns").len(), 1);
        // Any term absent → no hit (AND-semantics, not OR).
        assert!(search_results(&index, &stdlib, "hash xml").is_empty());
    }

    // ── The download ceiling refuses, it does not truncate ───────────────────────
    //
    // The defect these guard: the ceiling used to be `.take(50 MB)`, so an index
    // past it came back SHORT and passed for a complete document — the failure
    // surfaced as a JSON syntax error deep inside a package nobody had touched.
    // What is asserted is therefore the REFUSAL, not the size.

    #[test]
    fn a_body_at_the_ceiling_is_kept_whole() {
        let body = vec![b'x'; 1000];
        let got = read_capped(&body[..], 1000).expect("at the ceiling");
        assert_eq!(got.len(), 1000);
    }

    #[test]
    fn a_body_over_the_ceiling_is_refused_rather_than_truncated() {
        let body = vec![b'x'; 1000];
        let err =
            read_capped(&body[..], 999).expect_err("one byte over the ceiling must not succeed");
        assert!(
            err.contains("larger than the 999 B download ceiling"),
            "the message must name the ceiling it hit: {err}"
        );
        assert!(
            err.contains("LOFT_MAX_DOWNLOAD"),
            "the message must name the way past the ceiling: {err}"
        );
    }

    #[test]
    fn a_zero_ceiling_reads_without_limit() {
        let body = vec![b'x'; 4096];
        let got = read_capped(&body[..], 0).expect("0 = no ceiling");
        assert_eq!(got.len(), 4096);
    }

    // ── The derived trigger sidecar ──────────────────────────────────────────────

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("loft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn index_with_trigger(pkg: &str, trigger: &str, pad: &str) -> String {
        format!(
            r#"{{
            "schema_version": 1,
            "updated": "2026-08-20T00:00:00Z{pad}",
            "packages": {{
                "{pkg}": {{
                    "categories": [], "yanked": [],
                    "versions": {{
                        "0.1.0": {{
                            "url": "u", "sha256": "s", "size": 1, "loft": ">=0.8",
                            "triggers": ["{trigger}:text"],
                            "published": "2026-08-20T00:00:00Z"
                        }}
                    }}
                }}
            }}
        }}"#
        )
    }

    #[test]
    fn the_sidecar_is_built_from_the_index_and_then_answers_on_its_own() {
        let dir = scratch_dir("sidecar-build");
        let index = dir.join("index.json");
        let sidecar = dir.join("triggers.json");
        std::fs::write(&index, index_with_trigger("regex", "matches", "")).unwrap();

        // First ask: no sidecar yet, so the index is parsed and the sidecar written.
        let first = catalog_trigger_map_at(&sidecar, &index);
        assert_eq!(first.get("matches").map(String::as_str), Some("regex"));
        assert!(
            sidecar.exists(),
            "the first ask must leave a sidecar behind"
        );

        // Prove the second ask reads the SIDECAR and not the index: rewrite the
        // sidecar's answer, keeping its stamp, and watch that answer come back.
        // A reader that re-parsed the index would still say `regex`.
        let text = std::fs::read_to_string(&sidecar).unwrap();
        std::fs::write(&sidecar, text.replace("\"regex\"", "\"impostor\"")).unwrap();
        let second = catalog_trigger_map_at(&sidecar, &index);
        assert_eq!(
            second.get("matches").map(String::as_str),
            Some("impostor"),
            "the sidecar is the fast path; this ask must not have touched the index"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sidecar_stamped_for_a_different_index_is_rebuilt() {
        let dir = scratch_dir("sidecar-stale");
        let index = dir.join("index.json");
        let sidecar = dir.join("triggers.json");
        std::fs::write(&index, index_with_trigger("regex", "matches", "")).unwrap();
        catalog_trigger_map_at(&sidecar, &index);

        // The index moves on — a different package, and a different length, so the
        // stamp mismatches even for two writes inside one second.
        std::fs::write(&index, index_with_trigger("globbing", "matches", "   ")).unwrap();
        let rebuilt = catalog_trigger_map_at(&sidecar, &index);
        assert_eq!(
            rebuilt.get("matches").map(String::as_str),
            Some("globbing"),
            "a stale stamp must send the reader back to the index"
        );
        // And the rebuild is persisted, not recomputed on every ask.
        let text = std::fs::read_to_string(&sidecar).unwrap();
        assert!(text.contains("globbing"), "sidecar not rewritten: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_absent_index_yields_an_empty_map_and_writes_nothing() {
        let dir = scratch_dir("sidecar-absent");
        let index = dir.join("index.json");
        let sidecar = dir.join("triggers.json");
        assert!(catalog_trigger_map_at(&sidecar, &index).is_empty());
        assert!(
            !sidecar.exists(),
            "nothing was derived, so there is nothing to cache"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── loft#1045 — the cached index and its signature must swap as one ───────────
    //
    // A consumer's suite failed with "registry index signature INVALID — refusing
    // to install", a hard failure that `--allow-unsigned` cannot bypass, and the
    // identical command passed minutes later.  Nothing was wrong with the
    // signature: `~/.loft/registry/index.json` and its `.sig` were being replaced
    // while the run read them.

    /// The truncation half.  `std::fs::write` empties the file and then fills it,
    /// which on a 692 KB index leaves a window wide enough for another process to
    /// read a short file — so a replacement lands under a temporary name and is
    /// renamed into place instead.
    ///
    /// Every generation is one byte repeated, so a torn read shows up as a wrong
    /// length or as two different bytes in one copy.  The run asserts that it
    /// actually CROSSED a replacement in both directions; without that a clean
    /// result would only mean the reader never raced the writer.
    #[test]
    fn a_replaced_file_is_never_read_half_written() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        const SIZE: usize = 512 * 1024;
        let dir = scratch_dir("1045-atomic-write");
        let path = dir.join("index.json");
        std::fs::write(&path, vec![b'A'; SIZE]).expect("seed");

        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let path = path.clone();
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut flip = false;
                while !stop.load(Ordering::Relaxed) {
                    let byte = if flip { b'B' } else { b'A' };
                    replace_atomically(&path, &vec![byte; SIZE]).expect("replace");
                    flip = !flip;
                }
            })
        };

        let (mut seen_a, mut seen_b, mut torn, mut reads) = (false, false, 0usize, 0usize);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !(seen_a && seen_b && reads >= 200) {
            let got = std::fs::read(&path).expect("read");
            reads += 1;
            if got.len() != SIZE || got.iter().any(|&b| b != got[0]) {
                torn += 1;
                continue;
            }
            match got[0] {
                b'A' => seen_a = true,
                b'B' => seen_b = true,
                other => panic!("byte {other} belongs to no generation"),
            }
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().expect("writer");

        assert_eq!(
            torn, 0,
            "a reader saw a partially written file in {reads} reads"
        );
        assert!(
            seen_a && seen_b,
            "the reader never crossed a replacement in both directions \
             ({reads} reads) — a clean result here would prove nothing"
        );
    }

    /// A matched pair is taken on the first look, so the settle costs nothing on
    /// the path where nothing is wrong.
    #[test]
    fn a_matched_pair_is_accepted_on_the_first_look() {
        let dir = scratch_dir("1045-matched");
        let (content, sig) = (dir.join("index.json"), dir.join("index.json.sig"));
        std::fs::write(&content, b"gen2").expect("content");
        std::fs::write(&sig, b"gen2").expect("sig");

        let started = std::time::Instant::now();
        let settled =
            read_signed_pair_settling(&content, &sig, &mut |c, s| c == s).expect("read pair");

        assert!(settled.accepted);
        assert_eq!(settled.content, b"gen2");
        assert!(
            started.elapsed() < SETTLE_PAUSE,
            "an untorn pair waited on the settle"
        );
    }

    /// The shape from the field: the content is already the new generation while
    /// the signature is still the old one.  The writer finishes mid-settle, and
    /// the reader must come back with the matched pair rather than with the torn
    /// verdict that reads as tampering.
    ///
    /// The refresh is finished from inside the `accept` closure rather than by a
    /// thread racing a sleep.  `accept` runs once per attempt, AFTER both files
    /// are read, so completing the refresh on the first call places the second
    /// rename in exactly the window the field report describes — and places it
    /// there on every machine.  The thread it replaces slept for
    /// `SETTLE_PAUSE * 1.5` against a budget of `(SETTLE_ATTEMPTS - 1) *
    /// SETTLE_PAUSE`, i.e. 15 ms inside 40 ms; `sleep` guarantees a MINIMUM, so
    /// under a full `make ci` (24 parallel rustc compiles) the wakeup overshot
    /// and the gate went red on a machine, not on a defect.
    ///
    /// Nothing is lost by dropping the thread: what a concurrent writer buys is
    /// the atomicity of `replace_atomically`, and that is
    /// `a_replaced_file_is_never_read_half_written`'s job next door — which is
    /// robust because it exits on a CONDITION under a deadline instead of racing
    /// a fixed sleep.  What is under test here is the settle's re-read, and that
    /// is single-reader sequencing.
    #[test]
    fn a_pair_torn_by_a_refresh_settles_into_the_new_generation() {
        let dir = scratch_dir("1045-torn");
        let (content, sig) = (dir.join("index.json"), dir.join("index.json.sig"));
        std::fs::write(&content, b"gen2").expect("content");
        std::fs::write(&sig, b"gen1").expect("stale sig");

        let mut attempts = 0;
        let mut finish_the_refresh_after_the_first_read = |c: &[u8], s: &[u8]| {
            attempts += 1;
            if attempts == 1 {
                replace_atomically(&sig, b"gen2").expect("finish the refresh");
            }
            c == s
        };
        let settled =
            read_signed_pair_settling(&content, &sig, &mut finish_the_refresh_after_the_first_read)
                .expect("read pair");

        assert!(
            settled.accepted,
            "the settle gave up on a pair that was mid-refresh — the exact \
             false alarm loft#1045 reports"
        );
        assert_eq!(settled.content, b"gen2");
        assert_eq!(settled.signature, b"gen2");
        // Without this the test could pass by never having settled at all: one
        // attempt reads the torn pair, the second reads the finished one, and a
        // reader that skipped the re-read would answer on attempt 1.
        assert_eq!(
            attempts, 2,
            "the pair must be accepted on the SECOND attempt — the first reads it torn"
        );
    }

    /// A signature that genuinely does not match is still refused: the settle
    /// waits, the bytes never come to agree, and the verdict is unchanged.  This
    /// is the control for the test above — the fix must not turn a real rejection
    /// into a retry that eventually passes.
    #[test]
    fn a_signature_that_never_matches_is_still_refused() {
        let dir = scratch_dir("1045-refused");
        let (content, sig) = (dir.join("index.json"), dir.join("index.json.sig"));
        std::fs::write(&content, b"gen2").expect("content");
        std::fs::write(&sig, b"forged").expect("sig");

        let settled =
            read_signed_pair_settling(&content, &sig, &mut |c, s| c == s).expect("read pair");

        assert!(!settled.accepted);
        assert_eq!(settled.signature, b"forged");
    }

    /// Both files of a pair are replaced, neither left behind — and an empty
    /// signature leaves the cached one standing, which is the unsigned/bundle
    /// path where the caller has already accepted its absence.
    #[test]
    fn writing_a_pair_replaces_both_and_an_empty_signature_replaces_neither() {
        let dir = scratch_dir("1045-pair-write");
        let (content, sig) = (dir.join("index.json"), dir.join("index.json.sig"));

        write_signed_pair(&content, &sig, b"gen1", b"sig1").expect("first");
        assert_eq!(std::fs::read(&content).expect("content"), b"gen1");
        assert_eq!(std::fs::read(&sig).expect("sig"), b"sig1");

        write_signed_pair(&content, &sig, b"gen2", b"").expect("second");
        assert_eq!(std::fs::read(&content).expect("content"), b"gen2");
        assert_eq!(
            std::fs::read(&sig).expect("sig"),
            b"sig1",
            "an empty signature must not erase the cached one"
        );

        let strays: Vec<_> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(
            strays.is_empty(),
            "temporaries left beside the cache: {strays:?}"
        );
    }
}
