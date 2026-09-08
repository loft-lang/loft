// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! @PLN78 step 3 — decide what a self-update WOULD do, without doing any of it.
//!
//! The decision is a pure function of (index, running version, host triple), so it is
//! unit-testable against a synthetic index and needs no network, no release, and no
//! privileges.  That matters more than usual here: the registry does not yet carry a
//! toolchain entry (step 1b, an owner action), so this code cannot be exercised against
//! the real index — proving it against a constructed one is the only way to know it is
//! right BEFORE the entry is published, rather than discovering it afterwards.
//!
//! ## What this pins for step 1b
//!
//! Writing the resolver is what forces the entry's shape to be decided, so the
//! decisions live here as executable expectations rather than prose:
//!
//! * the toolchain is the package named [`TOOLCHAIN_PKG`] (`loft`);
//! * a release is installable on a host only if its `binaries` map has an entry for
//!   that **target triple** — a version that exists but was not built for you is
//!   reported as such, not silently skipped, because "no update available" and "no
//!   build for your platform" send a user to different places;
//! * yanked and prerelease versions are excluded by reusing
//!   [`registry_index::find_best_version`], not by a second rule that could drift;
//! * an update is only ever offered UPWARDS.  Everything else here is a report, but a
//!   downgrade is the one outcome that could hand a user a known-vulnerable release,
//!   so the direction is enforced in the planner rather than at the call site.

use crate::registry_index::{Package, RegistryIndex, compare_semver, find_best_version};
use std::path::{Path, PathBuf};

/// The registry package name that carries the loft toolchain itself.
pub const TOOLCHAIN_PKG: &str = "loft";

/// Download the release bundle a [`Plan::Available`] names, verify it, and unpack it.
///
/// Returns the staged bundle root — the directory holding `bin/` and `default/` — ready
/// for [`apply_bundle`].  Nothing on the installation is touched.
///
/// The hash comes from the plan, which came from the SIGNED index, so this is the step
/// where the signature stops being a claim about a JSON file and becomes a claim about
/// the bytes that are about to replace the running loft.  It is checked before the zip
/// is opened: a mismatched archive is not something to unpack and inspect, it is
/// something to refuse.
///
/// # Errors
/// Download failure, hash mismatch, a malformed archive, or an archive whose layout is
/// not a release bundle.
#[cfg(feature = "registry")]
pub fn fetch_bundle(url: &str, sha256: &str, tmp: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(tmp).map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
    let zip_path = tmp.join("bundle.zip");
    let bytes = crate::registry_index::download_tarball(url, &zip_path)?;
    crate::integrity::verify_sha256(&bytes, sha256).map_err(|e| {
        // Leave nothing unpacked behind that failed its hash.
        let _ = std::fs::remove_file(&zip_path);
        format!(
            "the downloaded bundle does not match the hash the signed index publishes \
             ({e}).  Nothing was installed."
        )
    })?;
    let extract = tmp.join("x");
    let file = std::fs::File::open(&zip_path).map_err(|e| format!("cannot open bundle: {e}"))?;
    zip::ZipArchive::new(file)
        .map_err(|e| format!("the bundle is not a readable zip: {e}"))?
        .extract(&extract)
        .map_err(|e| format!("cannot unpack the bundle: {e}"))?;
    staged_root(&extract).ok_or_else(|| {
        "the archive does not look like a release bundle (no bin/loft inside)".to_string()
    })
}

/// The bundle root inside an unpacked archive: the release zips wrap everything in one
/// `loft-<version>-<triple>/` directory, but an archive repacked without it is still a
/// bundle, so both layouts are accepted.
#[cfg(feature = "registry")]
fn staged_root(extract: &Path) -> Option<PathBuf> {
    if extract.join("bin").join("loft").is_file() || extract.join("bin").join("loft.exe").is_file()
    {
        return Some(extract.to_path_buf());
    }
    for entry in std::fs::read_dir(extract).ok()?.flatten() {
        let p = entry.path();
        if p.join("bin").join("loft").is_file() || p.join("bin").join("loft.exe").is_file() {
            return Some(p);
        }
    }
    None
}

/// What a self-update would do, decided but not done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// The index carries no toolchain entry at all — today's state, until step 1b.
    /// Distinct from `Current` so the report can say "nothing published yet" rather
    /// than implying the running version was checked and found newest.
    NoEntry,
    /// The running version is the newest installable one.
    Current { version: String },
    /// A newer release exists and was built for this host.
    Available {
        from: String,
        to: String,
        url: String,
        sha256: String,
    },
    /// A newer release exists but not for this target triple.  Reported rather than
    /// treated as "up to date", which would be a lie by omission.
    NoBuildForTarget {
        to: String,
        triple: String,
        built_for: Vec<String>,
    },
}

/// Decide what a self-update from `current` on `triple` would do against `index`.
///
/// Pure: no IO, no clock, no environment.
#[must_use]
pub fn plan(index: &RegistryIndex, current: &str, triple: &str) -> Plan {
    let Some(pkg) = index.packages.get(TOOLCHAIN_PKG) else {
        return Plan::NoEntry;
    };
    plan_for_package(pkg, current, triple)
}

fn plan_for_package(pkg: &Package, current: &str, triple: &str) -> Plan {
    // `"*"` = "any version"; `find_best_version` applies the yanked + prerelease rules,
    // which is why they are not restated here.
    let Some(best) = find_best_version(pkg, "*", false) else {
        return Plan::NoEntry;
    };
    // Upwards only.  A registry that offered an older release — through a rollback, a
    // mistake, or tampering that survived signing — must not be able to walk a user
    // back onto a version an advisory already covers.
    if compare_semver(&best.semver, current) != std::cmp::Ordering::Greater {
        return Plan::Current {
            version: current.to_string(),
        };
    }
    match best.binaries.get(triple) {
        Some(bin) => Plan::Available {
            from: current.to_string(),
            to: best.semver.clone(),
            url: bin.url.clone(),
            sha256: bin.sha256.clone(),
        },
        None => Plan::NoBuildForTarget {
            to: best.semver.clone(),
            triple: triple.to_string(),
            built_for: best.binaries.keys().cloned().collect(),
        },
    }
}

/// The triple naming the release artifact that applies to this host — the key a
/// toolchain entry's `binaries` map is looked up by.
///
/// This is the PUBLISHED triple, not the one this binary happened to be built with, and
/// the difference is load-bearing on Linux: releases ship `x86_64-unknown-linux-musl`
/// (static, so it runs on glibc systems too) while a local `cargo build` is
/// `-gnu`.  Returning the build triple would send every Linux user looking for an entry
/// that does not exist, and they would be told "published, but not built for your
/// platform" about the very artifact meant for them.  `scripts/install.sh` derives the
/// same name from `uname`, and [`PUBLISHED_TRIPLES`] pins the pair together.
#[must_use]
pub fn host_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        _ => format!("{arch}-unknown-linux-musl"),
    }
}

/// The target triples a loft release publishes, as they appear in the release assets.
///
/// Here so the lookup key and the installer cannot drift apart silently: a target added
/// to `make-release.sh` without a matching entry here is a target `self-update` will
/// never offer.
pub const PUBLISHED_TRIPLES: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-musl",
];

// ── @PLN78 step 5 — the advisory half ───────────────────────────────────────────
//
// The plan called 30 and @PLAN12 §6.7 "half a system" each: 6.7 produces the yank
// and advisory signals, and this is what reacts to them.
//
// The load-bearing choice is WHICH version to check.  Checking only the candidate
// would miss the case that matters most: a registry that is stalled, pinned, or
// simply has nothing newer offers no update at all, and a user sitting on a release
// with a known advisory would then be told "you are up to date" — technically true
// and exactly wrong.  So the RUNNING version is checked whether or not an update
// exists, and that is the loop closing.
//
// It reports; it does not restrict.  Whether to keep running a flagged release is
// the user's call on the user's machine, and a tool that refuses to start is one
// they will work around rather than heed.  What it owes them is the advisory id,
// what it is, and where the fix landed.

/// An advisory against a specific loft version, flattened for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flag {
    pub version: String,
    pub severity: String,
    pub id: String,
    pub summary: String,
    /// The release the fix landed in, when one is published.
    pub fixed_in: Option<String>,
}

/// Advisories against loft `version`, most severe first.
///
/// Pure, so the reporting rules are testable without a feed to fetch.
#[must_use]
pub fn flags_for(feed: &crate::registry_advisories::AdvisoryFeed, version: &str) -> Vec<Flag> {
    let mut hits: Vec<Flag> = crate::registry_advisories::classify(TOOLCHAIN_PKG, version, feed)
        .into_iter()
        .map(|c| Flag {
            version: c.version,
            severity: c.severity.as_str().to_string(),
            id: c.advisory_id,
            summary: c.summary,
            fixed_in: c.fixed_in,
        })
        .collect();
    // Most severe first: a critical must not be buried under a note.
    hits.sort_by_key(|f| {
        std::cmp::Reverse(
            crate::registry_advisories::classify(TOOLCHAIN_PKG, version, feed)
                .iter()
                .find(|c| c.advisory_id == f.id)
                .map_or(0, |c| c.severity.rank()),
        )
    });
    hits
}

// ── @PLN78 step 4 — applying a staged bundle ────────────────────────────────────
//
// The update unit is NOT a directory.  `verify_self::bundle_root` is
// `<binary-dir>/..`, which for a system install is a shared PREFIX — `/usr/local`,
// whose `bin/` holds every other binary on the machine.  Renaming or replacing that,
// or even `bin/`, would take unrelated software with it.
//
// So the unit is exactly the files the bundle CLAIMS: `SHA256SUMS` lists every file a
// release ships, which makes the bundle self-describing about what it owns.  Anything
// not in that list is untouched, by construction rather than by a rule someone has to
// remember.
//
// That set cannot be replaced atomically, and pretending otherwise would be the
// dangerous design.  Two things make it safe instead: nothing is moved until the
// staged bundle has verified against its OWN manifests, and every replaced file is
// backed up so a mid-swap failure restores.  The residual window — a crash between
// two file replacements — is exactly what `loft verify-self` detects and names, which
// is why step 2 came first.

/// Paths a bundle owns, read from its `SHA256SUMS` — the authoritative list of what a
/// release ships, and therefore of what an update may replace.
fn owned_files(staged: &Path) -> Result<(Vec<String>, bool), String> {
    let sums = staged.join("SHA256SUMS");
    // No manifest is not a refusal.  A release bundle always carries one — that rule is
    // enforced where we PUBLISH — but a user may have assembled or built this directory
    // themselves, and they pointed at it deliberately.  Then the bundle's contents are
    // the set: everything in it is installed, and nothing outside it is touched or
    // removed.  Refusing here would wall off precisely the people `--from` exists for.
    let Ok(text) = std::fs::read_to_string(&sums) else {
        return walk_files(staged).map(|f| (f, false));
    };
    let (entries, _) = crate::verify_self::parse_manifest(&text);
    if entries.is_empty() {
        return walk_files(staged).map(|f| (f, false));
    }
    for (rel, _) in &entries {
        // The one home of the escape rule (see `verify_self::manifest_path_escapes`);
        // this and `check_manifest` carried separate copies until they drifted.
        if crate::verify_self::manifest_path_escapes(rel) {
            return Err(format!("staged bundle lists an unsafe path: {rel}"));
        }
    }
    let mut files: Vec<String> = entries.into_iter().map(|(rel, _)| rel).collect();
    // `SHA256SUMS` cannot appear in its own list — a file cannot carry its own digest —
    // so the set it defines never includes it.  Left out, an update would install new
    // files under the OLD manifest and the result would fail `verify-self` forever.
    // The manifest that defines the set is the one thing the set cannot state.
    if !files.iter().any(|f| f == "SHA256SUMS") {
        files.push("SHA256SUMS".to_string());
    }
    Ok((files, true))
}

/// Every file under `staged`, as bundle-relative paths — the set for a directory that
/// carries no manifest of its own.
fn walk_files(staged: &Path) -> Result<Vec<String>, String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let path = e.path();
            // Never follow a symlink out of the bundle: the set must stay what the
            // directory actually contains.
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|err| format!("reading {}: {err}", path.display()))?;
            if meta.is_dir() {
                walk(&path, base, out)?;
            } else if meta.is_file()
                && let Ok(rel) = path.strip_prefix(base)
            {
                // `portable_path` and not a blind backslash replace: on Unix a
                // backslash is a legal filename character, and rewriting it would
                // rename someone's file on the way into the set.
                out.push(crate::portable_path::portable(rel));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(staged, staged, &mut out)?;
    if out.is_empty() {
        return Err("the bundle directory is empty".to_string());
    }
    out.sort();
    Ok(out)
}

/// Replace the files of the installation at `root` with those of the verified bundle at
/// `staged`, restoring every replaced file if any step fails.
///
/// Refuses before touching anything unless `staged` verifies against its own manifests:
/// an update that installs a bundle it could not vouch for is worse than no update.
///
/// # Errors
/// Returns `Err` (with the installation restored) when the staged bundle does not
/// verify, lists an unsafe path, or a file cannot be replaced.
pub fn apply_bundle(root: &Path, staged: &Path, force: bool) -> Result<Vec<String>, String> {
    // 1. The staged bundle should vouch for itself before anything moves — but this is
    //    the user's machine, so `force` is always available.  The strictness that
    //    matters is on the PUBLISHING side: we never ship a release that cannot be
    //    fully verified.  What a user chooses to install is theirs to decide, and a
    //    tool that refuses a bundle its owner wants is just an obstacle to route
    //    around.  The default protects against the accident (a truncated download, a
    //    half-copied directory); `force` covers the case where they mean it.
    if !force {
        for check in crate::verify_self::local_checks(staged) {
            if let crate::verify_self::Check::Failed(m) = check {
                return Err(format!("staged bundle failed its own manifest: {m}"));
            }
        }
    }
    let (files, described) = owned_files(staged)?;

    // 2. Replace each file, remembering how to put it back.
    let backup_dir = root.join(format!(".loft-update-backup-{}", std::process::id()));
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("creating {}: {e}", backup_dir.display()))?;
    let mut restore: Vec<(PathBuf, PathBuf)> = Vec::new(); // (target, backup)
    let mut placed: Vec<PathBuf> = Vec::new();
    let mut result = Ok(());
    for rel in &files {
        let target = root.join(rel);
        let source = staged.join(rel);
        if !source.is_file() {
            result = Err(format!("staged bundle is missing {rel}"));
            break;
        }
        if let Some(parent) = target.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            result = Err(format!("cannot create {}", parent.display()));
            break;
        }
        // Move the existing file aside rather than overwriting it: on Windows the
        // running executable cannot be overwritten but CAN be renamed, and the
        // rename is what makes a restore possible on every platform.
        if target.exists() {
            let backup = backup_dir.join(rel.replace(['/', '\\'], "__"));
            if let Err(e) = std::fs::rename(&target, &backup) {
                result = Err(format!("cannot move {rel} aside: {e}"));
                break;
            }
            restore.push((target.clone(), backup));
        }
        if let Err(e) = copy_file(&source, &target) {
            result = Err(format!("cannot place {rel}: {e}"));
            break;
        }
        placed.push(target);
    }

    // 3. A bundle with NO manifests leaves the installation's OLD ones describing files
    //    that no longer exist, so `verify-self` would report a permanent, meaningless
    //    failure for someone who deliberately installed their own build.  Retire them:
    //    "not a release bundle" is the truthful answer for what they now have.  Backed
    //    up like everything else, so a rollback restores them.
    if result.is_ok() && !described {
        let target = root.join(crate::verify_self::MANIFEST);
        if target.is_file() {
            let backup = backup_dir.join(crate::verify_self::MANIFEST);
            if std::fs::rename(&target, &backup).is_ok() {
                restore.push((target, backup));
            }
        }
    }

    // 4. And the result must verify — when there is something to verify it against.
    if result.is_ok() && described && !force {
        for check in crate::verify_self::local_checks(root) {
            if let crate::verify_self::Check::Failed(m) = check {
                result = Err(format!("updated installation failed verification: {m}"));
                break;
            }
        }
    }

    if let Err(e) = result {
        for p in &placed {
            let _ = std::fs::remove_file(p);
        }
        for (target, backup) in restore.iter().rev() {
            let _ = std::fs::rename(backup, target);
        }
        let _ = std::fs::remove_dir_all(&backup_dir);
        return Err(format!("{e} — the installation was restored"));
    }
    let _ = std::fs::remove_dir_all(&backup_dir);
    Ok(files)
}

/// Copy preserving the executable bit, which a plain byte copy loses — and a loft that
/// is not executable is not an installation.
fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::copy(source, target).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(source) {
            let _ = std::fs::set_permissions(
                target,
                std::fs::Permissions::from_mode(meta.permissions().mode()),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_index::{BinaryEntry, Package, RegistryIndex, Version};
    use std::collections::BTreeMap;

    fn version(semver: &str, triples: &[&str]) -> Version {
        let mut binaries = BTreeMap::new();
        for t in triples {
            binaries.insert(
                (*t).to_string(),
                BinaryEntry {
                    manifest_sha256: None,
                    url: format!("https://example/loft-{semver}-{t}.zip"),
                    sha256: format!("hash-{semver}-{t}"),
                    loft_ffi_fp: None,
                },
            );
        }
        Version {
            semver: semver.to_string(),
            url: String::new(),
            sha256: String::new(),
            size: 0,
            loft: "*".to_string(),
            api_compatible_with: None,
            data_compatible_with: None,
            deps: BTreeMap::new(),
            conflicts: Vec::new(),
            replaces: Vec::new(),
            provides: Vec::new(),
            triggers: Vec::new(),
            binaries,
            api: Vec::new(),
            prerelease: false,
            published: "2026-07-21T00:00:00Z".to_string(),
        }
    }

    fn index(versions: Vec<Version>, yanked: Vec<&str>) -> RegistryIndex {
        let mut vmap = BTreeMap::new();
        for v in versions {
            vmap.insert(v.semver.clone(), v);
        }
        let mut packages = BTreeMap::new();
        packages.insert(
            TOOLCHAIN_PKG.to_string(),
            Package {
                name: TOOLCHAIN_PKG.to_string(),
                description: None,
                homepage: None,
                categories: Vec::new(),
                yanked: yanked.into_iter().map(str::to_string).collect(),
                versions: vmap,
            },
        );
        RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages,
            skipped: Vec::new(),
        }
    }

    const T: &str = "x86_64-unknown-linux-gnu";

    /// Today's state, and it must be distinguishable from "you are up to date" —
    /// nothing has been published, so nothing was checked.
    #[test]
    fn an_index_without_a_toolchain_entry_is_not_up_to_date() {
        let empty = RegistryIndex {
            schema_version: 1,
            updated: String::new(),
            packages: BTreeMap::new(),
            skipped: Vec::new(),
        };
        assert_eq!(plan(&empty, "2026.7.2", T), Plan::NoEntry);
    }

    /// The ordinary offer.
    #[test]
    fn a_newer_release_built_for_this_host_is_offered() {
        let idx = index(
            vec![version("2026.7.2", &[T]), version("2026.8.0", &[T])],
            vec![],
        );
        let Plan::Available {
            from,
            to,
            url,
            sha256,
        } = plan(&idx, "2026.7.2", T)
        else {
            panic!("a newer release must be offered");
        };
        assert_eq!((from.as_str(), to.as_str()), ("2026.7.2", "2026.8.0"));
        assert!(url.contains("2026.8.0"), "{url}");
        assert_eq!(sha256, format!("hash-2026.8.0-{T}"));
    }

    /// Calendar versions must order as versions, not as text — "2026.10.0" is newer
    /// than "2026.9.0" even though it sorts earlier as a string.
    #[test]
    fn calendar_versions_order_numerically() {
        let idx = index(
            vec![version("2026.9.0", &[T]), version("2026.10.0", &[T])],
            vec![],
        );
        let Plan::Available { to, .. } = plan(&idx, "2026.9.0", T) else {
            panic!("2026.10.0 is newer than 2026.9.0");
        };
        assert_eq!(to, "2026.10.0");
    }

    /// Running the newest: no offer.
    #[test]
    fn the_newest_version_reports_current() {
        let idx = index(
            vec![version("2026.7.2", &[T]), version("2026.8.0", &[T])],
            vec![],
        );
        assert_eq!(
            plan(&idx, "2026.8.0", T),
            Plan::Current {
                version: "2026.8.0".to_string()
            }
        );
    }

    /// The direction guard: a registry offering only OLDER releases must never walk a
    /// user backwards, which is the one outcome that could restore a vulnerable build.
    #[test]
    fn an_older_registry_never_downgrades() {
        let idx = index(vec![version("2026.6.0", &[T])], vec![]);
        assert_eq!(
            plan(&idx, "2026.7.2", T),
            Plan::Current {
                version: "2026.7.2".to_string()
            },
            "a lower published version must not be offered as an update"
        );
    }

    /// A yanked newest is skipped — and the rule is `find_best_version`'s, not a copy.
    #[test]
    fn a_yanked_newest_is_not_offered() {
        let idx = index(
            vec![
                version("2026.7.2", &[T]),
                version("2026.8.0", &[T]),
                version("2026.9.0", &[T]),
            ],
            vec!["2026.9.0"],
        );
        let Plan::Available { to, .. } = plan(&idx, "2026.7.2", T) else {
            panic!("the newest non-yanked release must be offered");
        };
        assert_eq!(to, "2026.8.0", "the yanked 2026.9.0 must be skipped");
    }

    /// A release that exists but was not built for this host is its own answer:
    /// "no update" and "no build for your platform" send a user to different places.
    #[test]
    fn a_release_without_a_build_for_this_target_says_so() {
        let idx = index(
            vec![
                version("2026.7.2", &[T]),
                version("2026.8.0", &["aarch64-apple-darwin"]),
            ],
            vec![],
        );
        let Plan::NoBuildForTarget {
            to,
            triple,
            built_for,
        } = plan(&idx, "2026.7.2", T)
        else {
            panic!("must distinguish a missing build from being up to date");
        };
        assert_eq!(to, "2026.8.0");
        assert_eq!(triple, T);
        assert_eq!(built_for, vec!["aarch64-apple-darwin".to_string()]);
    }

    /// The lookup key must be a triple the release actually PUBLISHES.  It was not:
    /// Linux composed `-gnu` (the build triple) while releases ship `-musl`, so every
    /// Linux user would have been told "published, but not built for your platform"
    /// about the artifact meant for them.  Caught by writing the installer's own
    /// detection, not by reading the code.
    #[test]
    fn host_triple_is_one_the_release_publishes() {
        let t = host_triple();
        assert!(
            PUBLISHED_TRIPLES.contains(&t.as_str()),
            "{t} is not a published target — self-update would find no entry for this host"
        );
    }

    // ── @PLN78 step 4 — applying a bundle ────────────────────────────────────────

    /// Build a bundle at `dir` with the given files, then write the manifests that
    /// describe it — the same shape `make-release.sh` produces.
    fn bundle(dir: &Path, files: &[(&str, &str)]) {
        for (rel, body) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
        }
        use std::fmt::Write as _;
        let mut stdlib = String::new();
        for (rel, body) in files {
            if rel.starts_with("default/") {
                let _ = writeln!(
                    stdlib,
                    "{}  {rel}",
                    crate::integrity::sha256_hex(body.as_bytes())
                );
            }
        }
        let mut sums = stdlib.clone();
        for (rel, body) in files {
            if !rel.starts_with("default/") {
                let _ = writeln!(
                    sums,
                    "{}  {rel}",
                    crate::integrity::sha256_hex(body.as_bytes())
                );
            }
        }
        std::fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    }

    fn dirs(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join("loft-apply-tests").join(name);
        let _ = std::fs::remove_dir_all(&base);
        let (root, staged) = (base.join("root"), base.join("staged"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        (root, staged)
    }

    /// The ordinary update: every file the new bundle claims is replaced, and the
    /// result verifies.
    #[test]
    fn applying_a_bundle_replaces_the_files_it_claims() {
        let (root, staged) = dirs("ok");
        bundle(
            &root,
            &[("bin/loft", "OLD BINARY"), ("default/a.loft", "old\n")],
        );
        bundle(
            &staged,
            &[("bin/loft", "NEW BINARY"), ("default/a.loft", "new\n")],
        );
        let placed = apply_bundle(&root, &staged, false).expect("a verified bundle must apply");
        assert!(placed.iter().any(|f| f == "bin/loft"), "{placed:?}");
        assert_eq!(
            std::fs::read_to_string(root.join("bin/loft")).unwrap(),
            "NEW BINARY"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("default/a.loft")).unwrap(),
            "new\n"
        );
        assert!(
            !crate::verify_self::local_checks(&root)
                .iter()
                .any(crate::verify_self::Check::failed),
            "the updated installation must verify"
        );
    }

    /// THE property that makes this safe on a system install.  `<binary-dir>/..` is a
    /// shared prefix — `/usr/local`, whose `bin/` holds other people's binaries — so an
    /// update may touch only what the bundle's own manifest claims, and nothing else.
    #[test]
    fn a_file_the_bundle_does_not_claim_is_never_touched() {
        let (root, staged) = dirs("foreign");
        bundle(&root, &[("bin/loft", "OLD"), ("default/a.loft", "old\n")]);
        // Somebody else's binary, sharing the prefix.
        std::fs::write(root.join("bin/othertool"), "NOT OURS").unwrap();
        std::fs::create_dir_all(root.join("share")).unwrap();
        std::fs::write(root.join("share/unrelated.conf"), "keep me").unwrap();
        bundle(&staged, &[("bin/loft", "NEW"), ("default/a.loft", "new\n")]);
        apply_bundle(&root, &staged, false).expect("apply");
        assert_eq!(
            std::fs::read_to_string(root.join("bin/othertool")).unwrap(),
            "NOT OURS",
            "an unrelated binary in the same prefix must survive an update"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("share/unrelated.conf")).unwrap(),
            "keep me"
        );
    }

    /// A bundle that cannot vouch for itself is refused BEFORE anything moves — an
    /// update that installs what it could not verify is worse than no update.
    #[test]
    fn a_staged_bundle_that_fails_its_manifest_moves_nothing() {
        let (root, staged) = dirs("badstage");
        bundle(&root, &[("bin/loft", "OLD"), ("default/a.loft", "old\n")]);
        bundle(&staged, &[("bin/loft", "NEW"), ("default/a.loft", "new\n")]);
        // Corrupt the staged bundle after its manifest was written.
        std::fs::write(staged.join("default/a.loft"), "tampered\n").unwrap();
        let err =
            apply_bundle(&root, &staged, false).expect_err("a corrupt bundle must be refused");
        assert!(
            err.contains("staged bundle failed its own manifest"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("bin/loft")).unwrap(),
            "OLD",
            "nothing may move when the staged bundle is refused"
        );
    }

    /// Rollback: a bundle whose manifest claims a file it does not ship gets as far as
    /// replacing others, then restores every one of them.
    #[test]
    fn a_failure_part_way_restores_the_installation() {
        let (root, staged) = dirs("rollback");
        bundle(&root, &[("bin/loft", "OLD"), ("default/a.loft", "old\n")]);
        bundle(&staged, &[("bin/loft", "NEW"), ("default/a.loft", "new\n")]);
        // The manifest still lists it; the file is gone. Verification of the staged
        // bundle reports it missing, so this is refused up front — and the
        // installation is untouched either way, which is what must hold.
        std::fs::remove_file(staged.join("default/a.loft")).unwrap();
        let err = apply_bundle(&root, &staged, false).expect_err("a missing file must be refused");
        assert!(err.contains("missing"), "{err}");
        assert_eq!(
            std::fs::read_to_string(root.join("bin/loft")).unwrap(),
            "OLD"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("default/a.loft")).unwrap(),
            "old\n"
        );
    }

    /// A manifest path may not reach outside the installation it describes.
    #[test]
    fn a_bundle_listing_an_escaping_path_is_refused() {
        let (root, staged) = dirs("escape");
        bundle(&root, &[("bin/loft", "OLD")]);
        bundle(&staged, &[("bin/loft", "NEW")]);
        let mut sums = std::fs::read_to_string(staged.join("SHA256SUMS")).unwrap();
        sums.push_str("00  ../../etc/passwd\n");
        std::fs::write(staged.join("SHA256SUMS"), sums).unwrap();
        let err =
            apply_bundle(&root, &staged, false).expect_err("an escaping path must be refused");
        assert!(
            err.contains("unsafe path") || err.contains("escapes"),
            "{err}"
        );
    }

    /// A bundle with NO manifests still installs — this is the route for someone who
    /// built or assembled it themselves.  The rule that a release must be fully
    /// verifiable binds US at publish time; it is not a gate on what a user may install
    /// on their own machine.
    #[test]
    fn a_bundle_without_a_manifest_still_installs() {
        let (root, staged) = dirs("nomanifest");
        bundle(&root, &[("bin/loft", "OLD"), ("default/a.loft", "old\n")]);
        // Hand-assembled: files, no SHA256SUMS.
        for (rel, body) in [("bin/loft", "MINE"), ("default/a.loft", "mine\n")] {
            let p = staged.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
        }
        let placed = apply_bundle(&root, &staged, false)
            .expect("a manifest-less bundle must install, not be refused");
        assert_eq!(placed.len(), 2, "{placed:?}");
        assert_eq!(
            std::fs::read_to_string(root.join("bin/loft")).unwrap(),
            "MINE"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("default/a.loft")).unwrap(),
            "mine\n"
        );
    }

    /// Even manifest-less, the set is what the directory CONTAINS — a neighbour sharing
    /// the prefix is still never touched.
    #[test]
    fn a_manifest_less_bundle_still_touches_only_its_own_files() {
        let (root, staged) = dirs("nomanifest_foreign");
        bundle(&root, &[("bin/loft", "OLD")]);
        std::fs::write(root.join("bin/othertool"), "NOT OURS").unwrap();
        let p = staged.join("bin/loft");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "MINE").unwrap();
        apply_bundle(&root, &staged, false).expect("apply");
        assert_eq!(
            std::fs::read_to_string(root.join("bin/othertool")).unwrap(),
            "NOT OURS"
        );
    }

    /// `--force` is the user's escape hatch: a bundle that contradicts its own manifest
    /// is refused by default, and installed when they say they mean it.
    #[test]
    fn force_installs_a_bundle_that_contradicts_its_manifest() {
        let (root, staged) = dirs("forced");
        bundle(&root, &[("bin/loft", "OLD"), ("default/a.loft", "old\n")]);
        bundle(&staged, &[("bin/loft", "NEW"), ("default/a.loft", "new\n")]);
        std::fs::write(staged.join("default/a.loft"), "tampered\n").unwrap();
        assert!(
            apply_bundle(&root, &staged, false).is_err(),
            "the default must refuse a bundle that contradicts itself"
        );
        apply_bundle(&root, &staged, true).expect("--force must honour the user's decision");
        assert_eq!(
            std::fs::read_to_string(root.join("default/a.loft")).unwrap(),
            "tampered\n"
        );
    }

    // ── @PLN78 step 5 — advisories against the RUNNING version ───────────────────

    fn feed(json: &str) -> crate::registry_advisories::AdvisoryFeed {
        crate::registry_advisories::parse_advisories(json).expect("parse advisories")
    }

    const ADVISORIES: &str = r#"{
      "schema_version": 1, "updated": "2026-07-31T00:00:00Z", "retention_days": 365,
      "advisories": [
        { "id": "GHSA-aaaa", "severity": "security_critical",
          "summary": "store image adopted without verification",
          "published": "2026-07-01T00:00:00Z", "references": [],
          "packages": [{ "name": "loft", "affected": ">=2026.6.0, <2026.7.3",
                         "fixed_in": "2026.7.3" }] },
        { "id": "GHSA-bbbb", "severity": "bug",
          "summary": "formatter drops a trailing comma",
          "published": "2026-07-02T00:00:00Z", "references": [],
          "packages": [{ "name": "loft", "affected": ">=2026.7.0, <2026.7.3",
                         "fixed_in": "2026.7.3" }] }
      ]
    }"#;

    /// The case the step exists for: the user is RUNNING a flagged release.  A stalled
    /// registry offers no update, so checking only the candidate would say nothing.
    #[test]
    fn a_flagged_running_version_is_reported_with_its_fix() {
        let flags = flags_for(&feed(ADVISORIES), "2026.7.2");
        assert_eq!(flags.len(), 2, "{flags:?}");
        assert!(
            flags
                .iter()
                .any(|f| f.id == "GHSA-aaaa" && f.fixed_in.as_deref() == Some("2026.7.3")),
            "the fix version must reach the user: {flags:?}"
        );
    }

    /// Most severe first — a critical must not be buried under a cosmetic note.
    #[test]
    fn the_most_severe_advisory_is_reported_first() {
        let flags = flags_for(&feed(ADVISORIES), "2026.7.2");
        assert_eq!(flags[0].id, "GHSA-aaaa", "critical must lead: {flags:?}");
        assert_eq!(flags[0].severity, "security_critical");
    }

    /// A fixed release is silent — the whole point of publishing the fix.
    #[test]
    fn a_fixed_version_has_no_flags() {
        assert!(flags_for(&feed(ADVISORIES), "2026.7.3").is_empty());
    }

    /// And a release predating the affected range is silent too, so the check cannot
    /// simply be "warn on everything".
    #[test]
    fn a_version_outside_the_affected_range_has_no_flags() {
        assert!(flags_for(&feed(ADVISORIES), "2026.5.0").is_empty());
    }

    /// @PLN78 step 4 — the download half, where the signature stops being a claim about
    /// a JSON file and becomes a claim about the bytes replacing the running loft.
    ///
    /// The refusal is the point: a bundle whose hash does not match the signed index is
    /// not unpacked and inspected, it is declined with nothing written.  The pass case
    /// is carried alongside because a check that refuses everything would satisfy the
    /// refusal assertion on its own.
    #[test]
    #[cfg(feature = "registry")]
    fn a_bundle_whose_hash_the_index_does_not_name_is_never_unpacked() {
        use std::io::Write;
        let base = std::env::temp_dir().join("loft-fetch-bundle-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // A real release-shaped zip: bin/loft inside a `loft-<v>-<triple>/` wrapper.
        let zip_path = base.join("release.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        w.start_file("loft-9.9.9-test/bin/loft", opts).unwrap();
        w.write_all(b"#!/bin/sh\n").unwrap();
        w.finish().unwrap();

        let bytes = std::fs::read(&zip_path).unwrap();
        let good = crate::integrity::sha256_hex(&bytes);
        let url = format!("file://{}", zip_path.display());

        // Pass: the hash matches, and the staged root is where `bin/loft` actually is.
        let ok_dir = base.join("ok");
        let staged = fetch_bundle(&url, &good, &ok_dir).expect("a matching bundle unpacks");
        assert!(
            staged.join("bin").join("loft").is_file(),
            "{}",
            staged.display()
        );

        // Refusal: one wrong hash, nothing unpacked.
        let bad_dir = base.join("bad");
        let err = fetch_bundle(&url, &"0".repeat(64), &bad_dir)
            .expect_err("a bundle the index does not vouch for must be refused");
        assert!(err.contains("signed index"), "{err}");
        assert!(
            !bad_dir.join("x").exists(),
            "a refused bundle must leave nothing unpacked"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── @PLN156 phase 4 — adversarial input: what a download or a bundle CLAIMS may
    // point anywhere, and every cell asserts the same invariant: nothing lands outside
    // the directory the operation owns.  The manifest `..` cell above
    // (`a_bundle_listing_an_escaping_path_is_refused`) was the first of this family;
    // these are its siblings across the other untrusted inputs.

    /// A zip whose ENTRY NAMES escape — `../x`, an absolute path — must leave nothing
    /// outside the staging directory, whatever the extractor does with the entries
    /// themselves.  The hash MATCHES on purpose: traversal is a property of the
    /// archive's contents, which the hash from the signed index vouches for only
    /// against substitution, not against a malicious release.
    #[test]
    #[cfg(feature = "registry")]
    fn a_zip_entry_that_escapes_the_staging_directory_lands_nowhere() {
        use std::io::Write;
        let base = std::env::temp_dir().join("loft-zip-traversal-test");
        let _ = std::fs::remove_dir_all(&base);
        // The canary lives BESIDE the staging dir: the place `../` reaches from inside.
        let staging = base.join("stage");
        std::fs::create_dir_all(&staging).unwrap();
        let escaped = base.join("escaped-by-dotdot.txt");
        let absolute = std::env::temp_dir().join("loft-zip-traversal-absolute.txt");
        let _ = std::fs::remove_file(&absolute);

        let zip_path = base.join("evil.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        // A valid bundle shape around the hostile entries, so a refusal (fine) and a
        // sanitising extract (also fine) are both reachable outcomes.
        w.start_file("bin/loft", opts).unwrap();
        w.write_all(b"#!/bin/sh\n").unwrap();
        w.start_file("../../escaped-by-dotdot.txt", opts).unwrap();
        w.write_all(b"escaped").unwrap();
        w.start_file(absolute.to_str().unwrap(), opts).unwrap();
        w.write_all(b"escaped").unwrap();
        w.finish().unwrap();

        let bytes = std::fs::read(&zip_path).unwrap();
        let hash = crate::integrity::sha256_hex(&bytes);
        let url = format!("file://{}", zip_path.display());
        // Refusing the archive and extracting it sanitised are BOTH acceptable
        // verdicts; writing outside `staging` is the only wrong one.
        let _ = fetch_bundle(&url, &hash, &staging);
        assert!(
            !escaped.exists(),
            "a `../` zip entry escaped the staging directory: {}",
            escaped.display()
        );
        assert!(
            !absolute.exists(),
            "an absolute zip entry landed at its own path: {}",
            absolute.display()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The absolute-path sibling of the manifest `..` cell: `/etc/...` carries no `..`,
    /// so it needs its own refusal (and its own test — the guard tests the two shapes
    /// with separate predicates).
    #[test]
    fn a_bundle_listing_an_absolute_path_is_refused() {
        let (root, staged) = dirs("absolute");
        bundle(&root, &[("bin/loft", "OLD")]);
        bundle(&staged, &[("bin/loft", "NEW")]);
        let mut sums = std::fs::read_to_string(staged.join("SHA256SUMS")).unwrap();
        sums.push_str("00  /etc/loft-evil\n");
        std::fs::write(staged.join("SHA256SUMS"), sums).unwrap();
        let err =
            apply_bundle(&root, &staged, false).expect_err("an absolute path must be refused");
        assert!(
            err.contains("unsafe path") || err.contains("escapes"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("bin/loft")).unwrap(),
            "OLD",
            "nothing may move when the manifest lists an absolute path"
        );
    }

    /// A symlink entry pointing out of the archive: whether the extractor materialises
    /// it as a link, a file, or not at all, nothing may land outside the staging
    /// directory — and nothing may be WRITTEN THROUGH it to the link's target.
    #[test]
    #[cfg(all(feature = "registry", unix))]
    fn a_symlink_entry_pointing_outside_writes_nothing_there() {
        use std::io::Write;
        let base = std::env::temp_dir().join("loft-zip-symlink-test");
        let _ = std::fs::remove_dir_all(&base);
        let staging = base.join("stage");
        std::fs::create_dir_all(&staging).unwrap();
        let target_dir = base.join("outside");
        std::fs::create_dir_all(&target_dir).unwrap();
        let canary = target_dir.join("canary.txt");
        std::fs::write(&canary, "untouched").unwrap();

        let zip_path = base.join("link.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let plain: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        // Entry 1: a symlink (unix mode 0o120777) whose body names the outside dir.
        let link_opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().unix_permissions(0o120_777);
        w.start_file("bin", link_opts).unwrap();
        w.write_all(target_dir.to_str().unwrap().as_bytes())
            .unwrap();
        // Entry 2: a file UNDER the linked name — through a materialised link this
        // write would land in `outside/`.
        w.start_file("bin/loft", plain).unwrap();
        w.write_all(b"#!/bin/sh\n").unwrap();
        w.finish().unwrap();

        let bytes = std::fs::read(&zip_path).unwrap();
        let hash = crate::integrity::sha256_hex(&bytes);
        let url = format!("file://{}", zip_path.display());
        let _ = fetch_bundle(&url, &hash, &staging);
        assert_eq!(
            std::fs::read_to_string(&canary).unwrap(),
            "untouched",
            "a write travelled through a symlink entry out of the staging directory"
        );
        assert!(
            !target_dir.join("loft").exists(),
            "bin/loft was written through the symlink into {}",
            target_dir.display()
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
