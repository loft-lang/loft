// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! @PLAN12 Phase 6.7 — security advisory channel.
//!
//! Sibling to `index.json` in the registry, signed by the same
//! Ed25519 key.  Small + fast-refresh (24 h TTL) — surfaces
//! CVEs against published packages without waiting for the full
//! `index.json` refresh.
//!
//! Schema (see [`security.md`](../doc/claude/lib_plans/12-library-extraction/security.md)):
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "updated": "2026-05-31T12:00:00Z",
//!   "retention_days": 90,
//!   "advisories": [
//!     {
//!       "id": "GHSA-xxxx-yyyy-zzzz",
//!       "packages": [{"name": "web", "affected": ">=0.1.0, <0.1.2", "fixed_in": "0.1.2"}],
//!       "severity": "security_critical",
//!       "summary": "TLS bypass in ws_client_connect",
//!       "published": "2026-05-30T08:00:00Z",
//!       "references": ["https://github.com/loft-lang/loft-libs-net/security/advisories/..."]
//!     }
//!   ]
//! }
//! ```
//!
//! `"package": "loft"` entries cover the loft binary itself —
//! same schema; same classifier.  See
//! [`lib-plan 30 § Phase 30.4`](../doc/claude/lib_plans/78-loft-distribution/README.md)
//! for the binary-side flow.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![cfg(feature = "registry")]

use std::path::PathBuf;

use crate::json::{Parsed, parse as parse_json};
use crate::registry_index;
use crate::registry_signing::{self, VerifyResult};

/// TTL for the cached advisory feed.  24 h per the design — small
/// file (~kilobytes), 90-day retention upstream, cheap to refresh.
/// The full `index.json` uses a separate, longer TTL (CSC of the
/// existing PKG.REG layer).
pub const ADVISORIES_TTL_SECS: u64 = 24 * 60 * 60;

/// Severity tier carried by each advisory.  Drives the loft-binary
/// classifier output ([`Classification::action`]):
/// - `SecurityCritical` → refuse to run.
/// - `SecurityHigh` → warn loudly; non-zero exit only under
///   `--strict-security`.
/// - `SecurityLow` / `Bug` → one-line warning per run.
/// - `Deprecated` → one-line note (suppressed by daily cadence
///   state once implemented).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    SecurityCritical,
    SecurityHigh,
    SecurityLow,
    Bug,
    Deprecated,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecurityCritical => "security_critical",
            Self::SecurityHigh => "security_high",
            Self::SecurityLow => "security_low",
            Self::Bug => "bug",
            Self::Deprecated => "deprecated",
        }
    }

    /// Rank for "worst severity" reductions in `loft audit` exit-code
    /// computation.  Higher = worse.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            Self::Deprecated => 1,
            Self::Bug | Self::SecurityLow => 2,
            Self::SecurityHigh => 3,
            Self::SecurityCritical => 4,
        }
    }
}

fn parse_severity(s: &str) -> Result<Severity, String> {
    match s {
        "security_critical" => Ok(Severity::SecurityCritical),
        "security_high" => Ok(Severity::SecurityHigh),
        "security_low" => Ok(Severity::SecurityLow),
        "bug" => Ok(Severity::Bug),
        "deprecated" => Ok(Severity::Deprecated),
        other => Err(format!("unknown severity `{other}`")),
    }
}

/// A package + version-range pair affected by an advisory.
#[derive(Debug, Clone)]
pub struct AdvisoryPackage {
    pub name: String,
    /// loft.toml-style range expression (`>=0.1.0, <0.1.2`).
    pub affected: String,
    /// Version the fix landed in (informational; printed to the
    /// user as "fix: <pkg> >= <ver>").  `None` when no fix is yet
    /// published.
    pub fixed_in: Option<String>,
}

/// A single advisory row.
#[derive(Debug, Clone)]
pub struct Advisory {
    /// GHSA / CVE / similar identifier — opaque string used as
    /// the cross-reference key from [`crate::registry_index`]'s
    /// per-version typed `status.advisory` field.
    pub id: String,
    /// Packages (one or more) affected by this advisory.
    pub packages: Vec<AdvisoryPackage>,
    pub severity: Severity,
    pub summary: String,
    /// ISO-8601 UTC timestamp.
    pub published: String,
    /// External URLs (NVD, GHSA, GitHub Security Advisory, etc.).
    pub references: Vec<String>,
}

/// Parsed `advisories.json`.
#[derive(Debug, Clone)]
pub struct AdvisoryFeed {
    pub schema_version: u32,
    pub updated: String,
    pub retention_days: u32,
    pub advisories: Vec<Advisory>,
}

/// Match between a (name, version) tuple and an advisory entry.
#[derive(Debug, Clone)]
pub struct Classification {
    pub package: String,
    pub version: String,
    pub advisory_id: String,
    pub severity: Severity,
    pub summary: String,
    pub fixed_in: Option<String>,
    pub references: Vec<String>,
}

/// Parse the JSON-serialised advisory feed.
///
/// # Errors
/// Surfaces JSON parse errors + missing required fields with the
/// path that failed.
pub fn parse_advisories(content: &str) -> Result<AdvisoryFeed, String> {
    let parsed = parse_json(content).map_err(|e| format!("JSON parse error: {e:?}"))?;
    let Parsed::Object(root) = parsed else {
        return Err("advisories.json: top-level must be an object".to_string());
    };
    let mut schema_version: Option<u32> = None;
    let mut updated: Option<String> = None;
    let mut retention_days: Option<u32> = None;
    let mut advisories: Vec<Advisory> = Vec::new();
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
            "retention_days" => {
                if let Some(n) = v.as_i64() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        retention_days = Some(n as u32);
                    }
                }
            }
            "advisories" => {
                let Parsed::Array(arr) = v else {
                    return Err("advisories.json: `advisories` must be an array".to_string());
                };
                for entry in arr {
                    advisories.push(parse_advisory(entry)?);
                }
            }
            _ => {
                // Forward-compat: silently ignore unknown top-level keys.
            }
        }
    }
    let schema_version =
        schema_version.ok_or_else(|| "advisories.json: missing `schema_version`".to_string())?;
    if schema_version != 1 {
        return Err(format!(
            "advisories.json: schema_version {schema_version} unsupported — upgrade loft"
        ));
    }
    Ok(AdvisoryFeed {
        schema_version,
        updated: updated.unwrap_or_default(),
        retention_days: retention_days.unwrap_or(90),
        advisories,
    })
}

fn parse_advisory(val: &Parsed) -> Result<Advisory, String> {
    let Parsed::Object(fields) = val else {
        return Err("advisory entry: expected object".to_string());
    };
    let mut id: Option<String> = None;
    let mut packages: Vec<AdvisoryPackage> = Vec::new();
    let mut severity: Option<Severity> = None;
    let mut summary: Option<String> = None;
    let mut published: Option<String> = None;
    let mut references: Vec<String> = Vec::new();
    for (k, _, v) in fields {
        match k.as_str() {
            "id" => {
                if let Parsed::Str(s) = v {
                    id = Some(s.clone());
                }
            }
            "packages" => {
                let Parsed::Array(arr) = v else {
                    return Err("advisory `packages` must be an array".to_string());
                };
                for entry in arr {
                    packages.push(parse_advisory_package(entry)?);
                }
            }
            "severity" => {
                if let Parsed::Str(s) = v {
                    severity = Some(parse_severity(s)?);
                }
            }
            "summary" => {
                if let Parsed::Str(s) = v {
                    summary = Some(s.clone());
                }
            }
            "published" => {
                if let Parsed::Str(s) = v {
                    published = Some(s.clone());
                }
            }
            "references" => {
                if let Parsed::Array(arr) = v {
                    for entry in arr {
                        if let Parsed::Str(s) = entry {
                            references.push(s.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let id = id.ok_or_else(|| "advisory: missing `id`".to_string())?;
    let severity = severity.ok_or_else(|| format!("advisory `{id}`: missing `severity`"))?;
    let summary = summary.ok_or_else(|| format!("advisory `{id}`: missing `summary`"))?;
    let published = published.ok_or_else(|| format!("advisory `{id}`: missing `published`"))?;
    if packages.is_empty() {
        return Err(format!(
            "advisory `{id}`: must list at least one affected package"
        ));
    }
    Ok(Advisory {
        id,
        packages,
        severity,
        summary,
        published,
        references,
    })
}

fn parse_advisory_package(val: &Parsed) -> Result<AdvisoryPackage, String> {
    let Parsed::Object(fields) = val else {
        return Err("advisory package entry: expected object".to_string());
    };
    let mut name: Option<String> = None;
    let mut affected: Option<String> = None;
    let mut fixed_in: Option<String> = None;
    for (k, _, v) in fields {
        match k.as_str() {
            "name" => {
                if let Parsed::Str(s) = v {
                    name = Some(s.clone());
                }
            }
            "affected" => {
                if let Parsed::Str(s) = v {
                    affected = Some(s.clone());
                }
            }
            "fixed_in" => {
                if let Parsed::Str(s) = v {
                    fixed_in = Some(s.clone());
                }
            }
            _ => {}
        }
    }
    let name = name.ok_or_else(|| "advisory package: missing `name`".to_string())?;
    let affected =
        affected.ok_or_else(|| format!("advisory package `{name}`: missing `affected` range"))?;
    Ok(AdvisoryPackage {
        name,
        affected,
        fixed_in,
    })
}

/// Classify a single (name, version) tuple against the feed.
///
/// Returns one [`Classification`] per matching advisory — multiple
/// advisories can hit the same package; the caller prints them all
/// (one line each per the design's open-question recommendation).
#[must_use]
pub fn classify(name: &str, version: &str, feed: &AdvisoryFeed) -> Vec<Classification> {
    let mut out: Vec<Classification> = Vec::new();
    for adv in &feed.advisories {
        for pkg in &adv.packages {
            if pkg.name != name {
                continue;
            }
            if !registry_index::satisfies(version, &pkg.affected) {
                continue;
            }
            out.push(Classification {
                package: name.to_string(),
                version: version.to_string(),
                advisory_id: adv.id.clone(),
                severity: adv.severity,
                summary: adv.summary.clone(),
                fixed_in: pkg.fixed_in.clone(),
                references: adv.references.clone(),
            });
        }
    }
    out
}

// ── Local cache + URL ─────────────────────────────────────────────

/// URL for `advisories.json`.  Derived from the registry URL by
/// replacing the trailing `index.json` with `advisories.json` —
/// the design specifies the two live alongside each other in the
/// registry repo.
#[must_use]
pub fn advisories_url() -> String {
    let url = registry_index::registry_url();
    if let Some(stripped) = url.strip_suffix("index.json") {
        format!("{stripped}advisories.json")
    } else if url.ends_with('/') {
        format!("{url}advisories.json")
    } else {
        format!("{url}/advisories.json")
    }
}

/// Paths for the cached advisory feed + signature.
#[must_use]
pub fn advisories_paths() -> (PathBuf, PathBuf) {
    let dir = registry_index::cache_dir();
    (dir.join("advisories.json"), dir.join("advisories.json.sig"))
}

/// Returns `true` when the cached advisory feed is older than
/// [`ADVISORIES_TTL_SECS`] (or absent).  Mirrors
/// `install::index_stale` shape — caller uses mtime.
#[must_use]
pub fn cache_stale(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(age) = modified.elapsed() else {
        return true;
    };
    age.as_secs() > ADVISORIES_TTL_SECS
}

// ── Loader ────────────────────────────────────────────────────────

/// Options for [`load_or_fetch`].
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    /// Skip signature verification (bootstrap state — no embedded
    /// trust root).  Mirrors `InstallOptions::allow_unsigned`.
    pub allow_unsigned: bool,
    /// Cache-only mode.  Returns Err if the cache is absent or
    /// stale (no network attempt).
    pub offline: bool,
    /// Force re-fetch even if cache is fresh.
    pub refresh: bool,
}

/// Load the advisory feed from cache, refreshing from the registry
/// when the cache is stale (or absent) and we're not in offline
/// mode.
///
/// Returns `Ok(None)` when:
/// - The feed isn't yet hosted in the registry (HTTP 404).  Treat
///   as "no advisories" — feature isn't active yet on this
///   registry.
/// - Offline + cache absent.  Caller decides whether to warn or
///   refuse.
///
/// Returns `Ok(Some(feed))` on success, `Err` on signature
/// mismatch or parse error.
///
/// # Errors
/// Signature verification failure, JSON parse failure, or
/// I/O errors writing the cache.
pub fn load_or_fetch(opts: &LoadOptions) -> Result<Option<AdvisoryFeed>, String> {
    let (cache_path, sig_path) = advisories_paths();
    let url = advisories_url();

    let content_bytes: Vec<u8> = if opts.offline {
        if !cache_path.exists() {
            return Ok(None);
        }
        // Verify here too.  This was the one branch of the four that read the cached
        // feed and trusted it unchecked, which put the whole signature gate behind a
        // single flag: `--offline` and the bytes are whatever is on disk (loft#1048).
        read_cached_feed_verified(&cache_path, &sig_path, *opts)?
    } else if opts.refresh || cache_stale(&cache_path) {
        // Refresh.  404 → feature not hosted yet; treat as absent.
        match registry_index::fetch_index(&url) {
            Ok(fetched) => {
                // Verify BEFORE caching, as the index path does.  Writing first and
                // checking after leaves a REFUSED feed on disk, where it outlives the
                // run that fetched it: every later command reads those bytes against
                // the previous, still-valid signature and fails the same way, and no
                // retry can clear it (loft#1048, the shape `install.rs` already went
                // through).  An ABSENT signature falls back to the cached one — the
                // offline/bundle path — but the verdict still comes first.
                let sig_bytes = if fetched.signature.is_empty() {
                    std::fs::read(&sig_path).unwrap_or_default()
                } else {
                    fetched.signature.clone()
                };
                verify_feed(&fetched.content, &sig_bytes, *opts)?;
                // Verified: now it is safe to keep.  Atomically, and as a pair —
                // another process reading this cache mid-refresh must not get a half
                // feed or the new feed beside the old signature (loft#1045).
                registry_index::write_signed_pair(
                    &cache_path,
                    &sig_path,
                    &fetched.content,
                    &fetched.signature,
                )
                .map_err(|e| format!("cache advisories: {e}"))?;
                fetched.content
            }
            Err(_) if !cache_path.exists() => {
                // Network failed AND no cache — soft error; treat
                // as "no advisories available", caller decides what
                // to log.
                return Ok(None);
            }
            Err(_) => {
                // Network failed but cache exists — fall through
                // to using the cached copy.
                read_cached_feed_verified(&cache_path, &sig_path, *opts)?
            }
        }
    } else {
        read_cached_feed_verified(&cache_path, &sig_path, *opts)?
    };

    let text = std::str::from_utf8(&content_bytes)
        .map_err(|e| format!("advisories.json is not valid UTF-8: {e}"))?;
    Ok(Some(parse_advisories(text)?))
}

/// Read the cached feed against the cached signature, through one door.
///
/// Every branch that serves cached bytes goes through here, so "verified on every
/// load, even cache-hit paths" is a property of the code rather than of remembering
/// to call `verify_feed` at four sites — one of which did not (loft#1048).
///
/// It also inherits the torn-pair handling the index got in loft#1045: the feed and
/// its signature are two files and cannot be swapped in one step, so a reader
/// crossing a refresh can pick up one of each.  `read_signed_pair_settling` re-reads
/// before believing a rejection, with the signature itself as the oracle.
fn read_cached_feed_verified(
    cache_path: &std::path::Path,
    sig_path: &std::path::Path,
    opts: LoadOptions,
) -> Result<Vec<u8>, String> {
    let settled = registry_index::read_signed_pair_settling(cache_path, sig_path, &mut |c, s| {
        verify_feed(c, s, opts).is_ok()
    })
    .map_err(|e| format!("read cached advisories: {e}"))?;
    if !settled.accepted {
        verify_feed(&settled.content, &settled.signature, opts)?;
    }
    Ok(settled.content)
}

fn verify_feed(content: &[u8], sig: &[u8], opts: LoadOptions) -> Result<(), String> {
    let result = registry_signing::verify_index(content, sig);
    match result {
        VerifyResult::Valid => Ok(()),
        VerifyResult::NoTrustRoot | VerifyResult::MalformedSignature if opts.allow_unsigned => {
            Ok(())
        }
        VerifyResult::Invalid => {
            Err("advisories.json signature INVALID — refusing to load".to_string())
        }
        VerifyResult::NoTrustRoot => Err(
            "advisories.json unsigned and this loft binary has no embedded trust root; \
             pass --allow-unsigned to proceed"
                .to_string(),
        ),
        VerifyResult::MalformedSignature => {
            Err("advisories.json signature is malformed".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
  "schema_version": 1,
  "updated": "2026-05-31T12:00:00Z",
  "retention_days": 90,
  "advisories": [
    {
      "id": "GHSA-test-aaaa-bbbb",
      "packages": [{"name": "web", "affected": ">=0.1.0, <0.1.2", "fixed_in": "0.1.2"}],
      "severity": "security_critical",
      "summary": "TLS bypass in ws_client_connect",
      "published": "2026-05-30T08:00:00Z",
      "references": ["https://example.com/advisory"]
    },
    {
      "id": "GHSA-test-cccc-dddd",
      "packages": [{"name": "gridmesh", "affected": ">=0.1.0, <0.1.1", "fixed_in": "0.1.1"}],
      "severity": "bug",
      "summary": "Off-by-one in step_y",
      "published": "2026-05-29T08:00:00Z",
      "references": []
    }
  ]
}"#;

    #[test]
    fn parses_fixture() {
        let feed = parse_advisories(FIXTURE).expect("parse");
        assert_eq!(feed.schema_version, 1);
        assert_eq!(feed.retention_days, 90);
        assert_eq!(feed.advisories.len(), 2);
        assert_eq!(feed.advisories[0].id, "GHSA-test-aaaa-bbbb");
        assert_eq!(feed.advisories[0].severity, Severity::SecurityCritical);
        assert_eq!(feed.advisories[0].packages.len(), 1);
        assert_eq!(feed.advisories[0].packages[0].name, "web");
        assert_eq!(
            feed.advisories[0].packages[0].fixed_in.as_deref(),
            Some("0.1.2")
        );
    }

    #[test]
    fn classifies_affected_version() {
        let feed = parse_advisories(FIXTURE).unwrap();
        let hits = classify("web", "0.1.0", &feed);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].advisory_id, "GHSA-test-aaaa-bbbb");
        assert_eq!(hits[0].severity, Severity::SecurityCritical);
    }

    #[test]
    fn skips_unaffected_version() {
        let feed = parse_advisories(FIXTURE).unwrap();
        let hits = classify("web", "0.1.2", &feed);
        assert!(hits.is_empty(), "0.1.2 is the fix; should be silent");
    }

    #[test]
    fn skips_unrelated_package() {
        let feed = parse_advisories(FIXTURE).unwrap();
        let hits = classify("crypto", "0.1.0", &feed);
        assert!(hits.is_empty());
    }

    #[test]
    fn rejects_missing_severity() {
        let bad = r#"{
            "schema_version": 1,
            "advisories": [{"id":"X","packages":[{"name":"a","affected":">=0"}],
                            "summary":"s","published":"now"}]
        }"#;
        assert!(parse_advisories(bad).is_err());
    }

    #[test]
    fn severity_rank_orders_correctly() {
        assert!(Severity::SecurityCritical.rank() > Severity::SecurityHigh.rank());
        assert!(Severity::SecurityHigh.rank() > Severity::Bug.rank());
        assert!(Severity::Bug.rank() > Severity::Deprecated.rank());
    }

    #[test]
    fn advisories_url_derives_from_index_url() {
        // Default URL ends in `index.json` — should swap to `advisories.json`.
        // SAFETY: tests are single-threaded for the read-then-restore env
        // dance; production never sets these.
        let prev = std::env::var("LOFT_REGISTRY_URL").ok();
        // SAFETY: env mutation in tests
        unsafe {
            std::env::set_var(
                "LOFT_REGISTRY_URL",
                "https://example.com/registry/index.json",
            );
        }
        assert_eq!(
            advisories_url(),
            "https://example.com/registry/advisories.json"
        );
        // SAFETY: env restore
        unsafe {
            if let Some(p) = prev {
                std::env::set_var("LOFT_REGISTRY_URL", p);
            } else {
                std::env::remove_var("LOFT_REGISTRY_URL");
            }
        }
    }

    #[test]
    fn cache_stale_when_path_absent() {
        let bogus = std::path::Path::new("/tmp/loft-advisories-test-nonexistent-xyz");
        assert!(cache_stale(bogus));
    }
}
