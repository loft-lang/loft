// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! `loft package` — produce a publishable tarball from a `loft.toml`
//! package directory.
//!
//! PKG.REG R1 (phase R1 of [PKG_REGISTRY.md](../doc/claude/PKG_REGISTRY.md)).
//! Produces `<pkg>-<version>.tar.gz` plus prints the SHA-256 + byte size
//! the publisher uses when adding the version to `loft-lang/registry`'s
//! `index.json`.
//!
//! Tarball layout (per PKG_REGISTRY.md § Tarball format):
//!
//! ```text
//! <pkg>-<version>/
//! ├── loft.toml
//! ├── src/
//! │   └── <pkg>.loft
//! ├── native/             (optional — only if present locally)
//! │   ├── Cargo.toml
//! │   ├── build.rs
//! │   └── src/lib.rs
//! ├── tests/              (optional)
//! └── README.md           (optional)
//! ```
//!
//! Excluded by default: `.git/`, `target/`, `.loft/`, `node_modules/`,
//! `.vscode/`, `.idea/`, any `*.tar.gz` (so the output file doesn't
//! recursively eat itself), and any path component starting with `.`
//! that isn't an explicit allow-listed dotfile (none today).
//!
//! In a git repo, files git ignores (untracked + matched by `.gitignore` /
//! `.git/info/exclude` / `core.excludesfile`) are ALSO excluded — see
//! `git_ignored_set`.  This keeps a package built from a dirty working tree
//! (e.g. after `loft test` wrote gitignored scratch files like
//! `tests/_tmp_*.bin`) byte-identical to one built from a clean clone, which the
//! registry's gate-3 reproducible-build check requires.  Parsing is delegated to
//! `git` rather than re-implemented; outside a git repo only the hardcoded list
//! above applies.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![cfg(feature = "registry")]

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::package_layout::{git_ignored_set, is_excluded_entry};
use flate2::Compression;
use flate2::GzBuilder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

/// The three compatibility levels a package declares, each a promise on a
/// different axis a consumer can be hurt on.
///
/// Produced by [`declared_levels`], which is the ONE place the rule lives —
/// `loft package` and `loft publish` both call it rather than re-deriving it.
/// Design: `doc/claude/plans/library-compat-contract/README.md` step 5a.
#[derive(Debug, Clone)]
pub struct DeclaredLevels {
    /// `[package] loft` — which loft this needs.  A range (`">=0.8"`), because
    /// the platform is the one axis a library does not choose a single point on.
    pub loft: String,
    /// `[package] api_compatible_with` — the oldest own release this is still a
    /// drop-in for.  A bare version: it names an artifact that exists.
    pub api: String,
    /// `[package] data_compatible_with` — the oldest own release whose stored
    /// data this still reads.
    pub data: String,
}

/// A floor names ONE release, so it must be a bare `major.minor.patch` (an
/// optional `-prerelease` suffix is allowed).  A range would name a set, and a
/// set cannot be fetched and run — which is the entire reason these are real
/// versions rather than abstract epochs.
fn is_bare_version(v: &str) -> bool {
    let core = v.split('-').next().unwrap_or(v);
    let mut parts = core.split('.');
    let ok =
        |p: Option<&str>| p.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    ok(parts.next()) && ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Check `[package]` declares all three compatibility levels, each naming a real
/// version at or below `version`.
///
/// Required before a package may be **registered** — these three are what a
/// consumer needs to decide whether an upgrade is safe on each axis it can be
/// hurt on: the platform, its call sites, its stored data.  A library that
/// declares nothing has promised nothing, which is fine right up until it asks
/// the registry to distribute it to people who cannot ask it questions.
///
/// # Errors
///
/// Returns **every** problem found rather than the first.  An author fixing one
/// line at a time pays a full tag-and-release cycle per round trip, so a
/// one-at-a-time gate would be several cycles of the same two-line edit.
pub fn declared_levels(
    manifest: &crate::manifest::Manifest,
    version: &str,
) -> Result<DeclaredLevels, Vec<String>> {
    let mut problems = Vec::new();

    // A floor at or above the release being cut would claim compatibility with
    // something that does not exist yet.  For a FIRST release the only honest
    // value is the version itself — trivially true, and the natural bootstrap.
    let mut floor = |field: &str, value: Option<&String>, means: &str| -> Option<String> {
        match value {
            None => {
                problems.push(format!(
                    "`{field}` is not declared.  It is {means}.\n    \
                     For a first release the value is this release itself:\n      \
                     {field} = \"{version}\"\n    \
                     Otherwise name the oldest own release the claim still holds for."
                ));
                None
            }
            Some(v) if !is_bare_version(v) => {
                problems.push(format!(
                    "`{field} = \"{v}\"` is not a bare version.  A floor names ONE release \
                     (`\"0.3.0\"`), never a range — the claim is verified by fetching that \
                     release and running its own tests, and a range names nothing to fetch."
                ));
                None
            }
            Some(v)
                if crate::registry_index::compare_semver(v, version)
                    == std::cmp::Ordering::Greater =>
            {
                problems.push(format!(
                    "`{field} = \"{v}\"` is newer than this release ({version}), so it claims \
                     compatibility with a version that does not exist yet."
                ));
                None
            }
            Some(v) => Some(v.clone()),
        }
    };

    let api = floor(
        "api_compatible_with",
        manifest.api_compatible_with.as_ref(),
        "the oldest release of this package whose public API this one is still a drop-in for",
    );
    let data = floor(
        "data_compatible_with",
        manifest.data_compatible_with.as_ref(),
        "the oldest release of this package whose stored or wire data this one still reads",
    );
    let loft = manifest.loft_version.clone();
    if loft.is_none() {
        problems.push(format!(
            "`loft` is not declared.  It is which loft this package needs, and unlike the \
             other two it is a RANGE — the loft you build and test it with:\n      \
             loft = \">={}\"",
            env!("CARGO_PKG_VERSION")
        ));
    }

    match (loft, api, data) {
        (Some(loft), Some(api), Some(data)) if problems.is_empty() => {
            Ok(DeclaredLevels { loft, api, data })
        }
        _ => Err(problems),
    }
}

/// Render a [`declared_levels`] failure as the message a publisher acts on.
///
/// Shared so `loft package` and `loft publish` refuse in the same words — a gate
/// that reads differently depending on which command hit it teaches that it is
/// two rules rather than one.
#[must_use]
pub fn declared_levels_error(name: &str, version: &str, problems: &[String]) -> String {
    let mut out = format!(
        "`{name}` {version} cannot be registered: a package must declare all three \
         compatibility levels first.\n\n"
    );
    for p in problems {
        out.push_str("  • ");
        out.push_str(p);
        out.push_str("\n\n");
    }
    out.push_str(
        "  These are the three axes an upgrade can hurt a consumer on — the platform, its\n  \
         call sites, its stored data — and a consumer cannot ask your package a question.\n  \
         Declaring a floor is what enters the contract; raising one later is how you\n  \
         declare a break, and it should read like the promise withdrawal it is.\n  \
         See doc/claude/COMPATIBILITY.md and `loft compat --help`.",
    );
    out
}

/// Result of a successful `package_create`.
#[derive(Debug)]
pub struct PackageOutput {
    /// Path to the generated tarball.
    pub tarball: PathBuf,
    /// Byte length of the tarball file.
    pub size: u64,
    /// Lowercase hex SHA-256 of the tarball bytes.
    pub sha256: String,
    /// Package name (from `[package] name`).
    pub name: String,
    /// Package version (from `[package] version`).
    pub version: String,
    /// Publishing repository (from `[package] repository`), if set.  Present →
    /// the release lives at that repo.  See [`release_url`].
    pub repository: Option<String>,
    /// `owner/repo` read from `git remote get-url origin` in the package
    /// directory, when there is one.  The fallback `[package] repository` does
    /// not have to be declared for, and the SAME fact `loft publish` derives —
    /// so the two commands cannot disagree about where a release lives
    /// (loft#1083).
    pub origin_repo: Option<String>,
    /// The three declared compatibility levels, when the manifest declares all
    /// of them (see [`declared_levels`]).  `None` → the package has not entered
    /// the contract, and `print_summary` refuses to emit a registry entry for it.
    pub levels: Option<DeclaredLevels>,
}

/// Walk `pkg_dir`, build a gzipped tarball, write it to disk, and
/// return its hash + size.  The output path is
/// `<pkg>-<version>.tar.gz` in `out_dir` (defaults to `pkg_dir`).
///
/// # Errors
///
/// Returns `io::Error` on:
/// - Missing or unreadable `loft.toml`.
/// - Manifest missing `[package] name` or `version`.
/// - Filesystem walk / archive write failure.
pub fn package_create(pkg_dir: &Path, out_dir: Option<&Path>) -> io::Result<PackageOutput> {
    let manifest_path = pkg_dir.join("loft.toml");
    let manifest =
        crate::manifest::read_manifest(manifest_path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 manifest path")
        })?)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("loft.toml not found at {}", manifest_path.display()),
            )
        })?;
    // Cloned rather than moved out: `declared_levels` below needs the whole
    // manifest, and a partial move would make it unborrowable.
    let name = manifest.name.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "loft.toml is missing [package] name",
        )
    })?;
    let version = manifest.version.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "loft.toml is missing [package] version",
        )
    })?;

    let archive_prefix = format!("{name}-{version}");
    let out_name = format!("{archive_prefix}.tar.gz");
    let out_path = out_dir.unwrap_or(pkg_dir).join(&out_name);

    // Build the tarball.
    //
    // Determinism: same source bytes → same tarball, regardless of who
    // ran `loft package` or when.  The registry's gate-3 reproducible-
    // build re-check relies on this.  We normalise at two layers:
    //
    //   1. The OUTER gzip stream uses `GzBuilder::mtime(0)` so the
    //      gzip-mtime header byte field is fixed, not the wall clock.
    //   2. The INNER tar entries are written with manually-constructed
    //      headers that pin `mtime = 0`, `uid = 0`, `gid = 0`, owner
    //      strings empty, and mode = 0o644 (files) / 0o755 (dirs).
    //      `tar::Builder::append_path_with_name` defaults to copying
    //      the on-disk mtime + permissions, which leak through git's
    //      lack-of-mtime-preservation across clones.
    //
    // Files on disk may have arbitrary mtimes (commit time on one
    // checkout, clone time on another) — that's git's contract.  By
    // overriding both gzip + tar timestamps we make `loft package`
    // genuinely content-addressed: the tarball bytes depend only on
    // the file *contents* and the archive paths, nothing else.
    {
        let tar_gz = fs::File::create(&out_path)?;
        let enc = GzBuilder::new()
            .mtime(0)
            .write(tar_gz, Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.follow_symlinks(false);
        add_dir_contents(&mut builder, pkg_dir, &archive_prefix)?;
        let enc = builder.into_inner()?;
        enc.finish()?;
    }

    // Hash + size.
    let (size, sha256) = hash_file(&out_path)?;

    // Carried, not enforced: building a tarball is mechanical (the
    // reproducible-build check re-packages every library just to compare bytes,
    // and must not care what any of them declares).  The gate is at the two
    // commands that produce a REGISTRY ENTRY — `loft package` and `loft publish`.
    let levels = declared_levels(&manifest, &version).ok();

    Ok(PackageOutput {
        tarball: out_path,
        size,
        sha256,
        name,
        version,
        repository: manifest.repository,
        origin_repo: git_remote_org_repo(pkg_dir).map(|(o, r)| format!("{o}/{r}")),
        levels,
    })
}

/// Parse `git remote get-url origin` in `pkg_path` for the GitHub `owner/repo`.
/// Handles `https://github.com/<org>/<repo>(.git)?` and
/// `git@github.com:<org>/<repo>(.git)?`; `None` when the remote is absent or is
/// not a GitHub URL.
///
/// The ONE home for that derivation: `loft publish` reads it for the release
/// lookup and the homepage, and [`release_url`] reads it as the fallback when
/// `[package] repository` is undeclared.  They used to derive the release url
/// independently and disagreed for every package in a monorepo that declares no
/// `repository` — `loft package` guessed a `loft-<name>` repo that does not
/// exist (loft#1083).
#[must_use]
pub fn git_remote_org_repo(pkg_path: &Path) -> Option<(String, String)> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(pkg_path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?.trim().to_string();
    let stripped = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let stripped = stripped.strip_suffix(".git").unwrap_or(stripped);
    let (org, repo) = stripped.split_once('/')?;
    if org.is_empty() || repo.is_empty() {
        return None;
    }
    Some((org.to_string(), repo.to_string()))
}

/// Where the release tarball for `out` will live — the `url` of its registry
/// index entry.
///
/// The tag is always `<name>-v<version>`, which is what `loft publish` tags and
/// what the registry's published entries carry; a bare `[package] repository`
/// is org-relative under `loft-lang`, a value with a `/` is `owner/repo`.  When
/// the manifest declares no `repository` the package's own `origin` remote
/// answers, so a monorepo package needs no extra manifest line to get a correct
/// url.
///
/// `None` when neither source can name a repo.  That is a REFUSAL, not a guess:
/// the previous fallback invented `loft-lang/loft-<name>`, a repo that exists
/// for no package in any `loft-libs-*` monorepo, and the registry's own
/// `validate.py` only caught it by fetching the url and failing (loft#1083).
#[must_use]
pub fn release_url(out: &PackageOutput) -> Option<String> {
    let tarball_name = out
        .tarball
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let owner_repo = match out.repository.as_deref() {
        Some(repo) if repo.contains('/') => repo.to_string(),
        Some(repo) => format!("loft-lang/{repo}"),
        None => out.origin_repo.clone()?,
    };
    Some(format!(
        "https://github.com/{owner_repo}/releases/download/{}-v{}/{tarball_name}",
        out.name, out.version
    ))
}

/// Recursively add the contents of `src_dir` to `builder`, prefixing each entry with
/// `archive_prefix/`.  Skips whatever [`is_excluded_entry`] excludes — which covers the
/// tarball-in-progress, so a `loft package` run targeting its own directory does not bundle
/// the partially-written archive into itself.
fn add_dir_contents(
    builder: &mut tar::Builder<GzEncoder<fs::File>>,
    src_dir: &Path,
    archive_prefix: &str,
) -> io::Result<()> {
    let ignored = git_ignored_set(src_dir);
    walk(builder, src_dir, src_dir, archive_prefix, &ignored)
}

fn walk(
    builder: &mut tar::Builder<GzEncoder<fs::File>>,
    root: &Path,
    current: &Path,
    archive_prefix: &str,
    ignored: &std::collections::HashSet<PathBuf>,
) -> io::Result<()> {
    // Collect + sort entries so the resulting tarball is deterministic
    // across runs (same bytes → same SHA-256, which the publisher's
    // PR-validation script depends on).
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(current)?.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        let file_type = entry.file_type()?;

        // The shared include rule — git-ignored entries, `EXCLUDED_DIRS` at any depth, and
        // tar artefacts (the tarball being written, and any stale one).  `loft install <dir>`
        // reads the same rule via `copy_package_tree`, so the two cannot drift (loft#667).
        if is_excluded_entry(root, &path, name_str.as_ref(), file_type.is_dir(), ignored) {
            continue;
        }

        // Compute the archive-relative path: `<prefix>/<rel>`.
        let rel = path
            .strip_prefix(root)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let archive_rel = PathBuf::from(archive_prefix).join(rel);

        if file_type.is_dir() {
            walk(builder, root, &path, archive_prefix, ignored)?;
        } else if file_type.is_file() {
            // Deterministic file entry: hand-construct the tar header
            // with mtime=0, uid=0, gid=0, mode=0o644, empty owner
            // strings.  Skips the `append_path_with_name` defaults
            // which copy on-disk mtime + uid + gid — those vary
            // between checkouts (git doesn't preserve mtimes) and
            // would make the tarball non-reproducible.
            let metadata = path.metadata()?;
            let mut header = tar::Header::new_gnu();
            header.set_size(metadata.len());
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            header.set_entry_type(tar::EntryType::Regular);
            // `username` / `groupname` default to empty for new headers.
            // `set_cksum()` must be called LAST — it computes the
            // checksum over every other header byte, so any later
            // mutation invalidates it.
            let file = fs::File::open(&path)?;
            builder.append_data(&mut header, &archive_rel, file)?;
        }
        // Symlinks: skipped here — a symlinked file caught by
        // `is_file()` above goes through the deterministic-header
        // path; a true symlink (`file_type.is_symlink()` true,
        // `is_file()` false) is dropped silently.  Packages should
        // not rely on symlinks in the tarball; the MVP behaviour is
        // unchanged for the regular-file case.
    }
    Ok(())
}

/// Read `path` into memory, return `(byte_count, lowercase_hex_sha256)`.
/// Tarballs in the MVP are typically <100 kB; the in-memory hash is
/// fine.  Switch to streaming if a publish target exceeds ~10 MB.
fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let size = buf.len() as u64;
    let mut hasher = Sha256::new();
    hasher.update(&buf);
    let digest = hasher.finalize();
    let hex = hex_encode(&digest);
    Ok((size, hex))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Format a `PackageOutput` for human consumption — the surface the
/// publisher reads to know what to paste into the registry index PR.
///
/// # Errors
///
/// Propagates any `io::Error` from the underlying writer.
#[allow(clippy::cast_precision_loss)] // package sizes fit comfortably in f64
pub fn print_summary(out: &PackageOutput, w: &mut dyn Write) -> io::Result<()> {
    writeln!(w, "{}", out.tarball.display())?;
    writeln!(w, "  package:  {} v{}", out.name, out.version)?;
    writeln!(
        w,
        "  size:     {} bytes ({:.1} kB)",
        out.size,
        out.size as f64 / 1024.0
    )?;
    writeln!(w, "  sha256:   {}", out.sha256)?;
    writeln!(w)?;
    writeln!(
        w,
        "Index entry to paste into loft-lang/registry/index.json (PKG_REGISTRY.md schema):"
    )?;
    writeln!(w, "  \"{}\": {{", out.version)?;
    // ONE derivation, shared with `loft publish` — see `release_url`.
    let Some(url) = release_url(out) else {
        writeln!(
            w,
            "Cannot name the release url: `[package] repository` is undeclared and this\n\
             directory has no GitHub `origin` remote.  Add `repository = \"<repo>\"` to\n\
             [package], or run `loft publish --dry-run` from the checkout."
        )?;
        return Ok(());
    };
    writeln!(w, "    \"url\": \"{url}\",")?;
    writeln!(w, "    \"sha256\": \"{}\",", out.sha256)?;
    writeln!(w, "    \"size\": {},", out.size)?;
    // The manifest's own value, not a constant.  This line read a hardcoded
    // `">=0.8"` for every package regardless of what it declared, so a library
    // needing a newer loft published an entry saying it did not.
    let levels = out.levels.as_ref();
    writeln!(
        w,
        "    \"loft\": \"{}\",",
        levels.map_or(">=0.8", |l| l.loft.as_str())
    )?;
    if let Some(l) = levels {
        // Carried into the index so a resolver can read a version's promises
        // without fetching and unpacking its tarball first.
        writeln!(w, "    \"api_compatible_with\": \"{}\",", l.api)?;
        writeln!(w, "    \"data_compatible_with\": \"{}\",", l.data)?;
    }
    writeln!(w, "    \"published\": \"<ISO-8601 UTC timestamp>\"")?;
    writeln!(w, "  }}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // The parity test lives here because this is where the TARBALL side is; the install
    // side moved to `package_layout` so it compiles without the `registry` feature.
    use crate::package_layout::copy_package_tree;
    use std::env;

    /// Per-test tmpdir.  Includes the test name + process id so
    /// parallel test runners don't trample each other's fixtures.
    fn tmpdir(name: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("loft_pkg_test_{}_{}", std::process::id(), name));
        if p.exists() {
            let _ = fs::remove_dir_all(&p);
        }
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Build a manifest from `[package]` body lines, so each level case differs
    /// in exactly the line under test.
    fn manifest_with(package_lines: &str) -> crate::manifest::Manifest {
        let dir = tmpdir(&format!(
            "levels_{:x}",
            package_lines.len() * 31
                + usize::from(package_lines.as_bytes().first().copied().unwrap_or(0))
        ));
        let path = dir.join("loft.toml");
        write(
            &path,
            &format!(
                "[package]\nname = \"probe\"\nversion = \"0.4.0\"\n{package_lines}\n\
                 [library]\nentry = \"src/probe.loft\"\n"
            ),
        );
        let m = crate::manifest::read_manifest(path.to_str().unwrap()).unwrap();
        let _ = fs::remove_dir_all(&dir);
        m
    }

    /// Step 5a: all three levels are required before a package may be
    /// registered, and each floor must name a version that could exist.
    ///
    /// The `all` / `first_release` cells are the ones that must PASS — without
    /// them a gate that rejected everything would look identical to a correct
    /// one, which is how the first hand-run of this matrix read.
    #[test]
    fn declared_levels_matrix() {
        let ok = |lines: &str| declared_levels(&manifest_with(lines), "0.4.0");
        let err = |lines: &str| ok(lines).expect_err("should be rejected").join(" | ");

        // All three declared, floors below the release being cut.
        let levels = ok(
            "loft = \">=0.8\"\napi_compatible_with = \"0.2.0\"\ndata_compatible_with = \"0.1.0\"",
        )
        .expect("all three declared");
        assert_eq!(levels.loft, ">=0.8");
        assert_eq!(levels.api, "0.2.0");
        assert_eq!(levels.data, "0.1.0");

        // A FIRST release: the floors are the release itself.  This is the
        // bootstrap the error message tells authors to write, so it has to pass.
        assert!(
            ok("loft = \">=0.8\"\napi_compatible_with = \"0.4.0\"\ndata_compatible_with = \"0.4.0\"")
                .is_ok()
        );

        // Each one missing is named, and its own name appears in the message —
        // an author must not have to guess which of the three is absent.
        assert!(
            err("loft = \">=0.8\"\ndata_compatible_with = \"0.1.0\"")
                .contains("`api_compatible_with` is not declared")
        );
        assert!(
            err("loft = \">=0.8\"\napi_compatible_with = \"0.2.0\"")
                .contains("`data_compatible_with` is not declared")
        );
        assert!(
            err("api_compatible_with = \"0.2.0\"\ndata_compatible_with = \"0.1.0\"")
                .contains("`loft` is not declared")
        );

        // ALL problems at once, not the first: each round trip costs a publish
        // cycle, so a one-at-a-time gate is several cycles of the same edit.
        let none = ok("").expect_err("nothing declared");
        assert_eq!(none.len(), 3, "expected all three reported, got: {none:?}");

        // A floor names ONE release, so a range is rejected — a set names
        // nothing that can be fetched and run.
        assert!(
            err("loft = \">=0.8\"\napi_compatible_with = \">=0.2\"\ndata_compatible_with = \"0.1.0\"")
                .contains("not a bare version")
        );
        // `loft` is the exception: it IS a range, and must stay accepted as one.
        assert!(ok("loft = \">=0.8\"\napi_compatible_with = \"0.2.0\"\ndata_compatible_with = \"0.1.0\"").is_ok());

        // A floor above the release claims compatibility with a version that
        // does not exist yet.
        assert!(
            err("loft = \">=0.8\"\napi_compatible_with = \"0.9.0\"\ndata_compatible_with = \"0.1.0\"")
                .contains("does not exist yet")
        );
    }

    /// The index entry carries the manifest's OWN `loft` range and both floors.
    /// This line was a hardcoded `">=0.8"` for every package regardless of what
    /// it declared, so a library needing a newer loft published an entry saying
    /// it did not.
    #[test]
    fn index_entry_carries_declared_levels() {
        let out = PackageOutput {
            tarball: PathBuf::from("crypto-0.2.1.tar.gz"),
            size: 1,
            sha256: "00".to_string(),
            name: "crypto".to_string(),
            version: "0.2.1".to_string(),
            repository: Some("loft-libs-core".to_string()),
            origin_repo: None,
            levels: Some(DeclaredLevels {
                loft: ">=2026.7".to_string(),
                api: "0.2.0".to_string(),
                data: "0.1.0".to_string(),
            }),
        };
        let mut buf = Vec::new();
        print_summary(&out, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("\"loft\": \">=2026.7\""), "{text}");
        assert!(
            text.contains("\"api_compatible_with\": \"0.2.0\""),
            "{text}"
        );
        assert!(
            text.contains("\"data_compatible_with\": \"0.1.0\""),
            "{text}"
        );
    }

    #[test]
    fn packages_minimal_layout() {
        let dir = tmpdir("packages_minimal_layout");
        let pkg = dir.join("hello");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n[library]\nentry = \"src/hello.loft\"\n",
        );
        write(
            &pkg.join("src").join("hello.loft"),
            "pub fn greet() -> text { return \"hi\"; }\n",
        );

        let out = package_create(&pkg, None).expect("package_create");
        assert_eq!(out.name, "hello");
        assert_eq!(out.version, "0.1.0");
        assert!(out.tarball.exists());
        assert!(out.size > 0);
        assert_eq!(out.sha256.len(), 64);
        assert!(out.sha256.chars().all(|c| c.is_ascii_hexdigit()));

        let _ = fs::remove_dir_all(&dir);
    }

    /// loft#1083 — `loft package` and `loft publish` must not name different
    /// urls for the same package.  `[package] repository` answers first; with no
    /// `repository` the `origin` remote answers, in the SAME `<name>-v<version>`
    /// form `loft publish` tags; with neither, the url is REFUSED rather than
    /// guessed as `loft-lang/loft-<name>` (a repo no monorepo package has).
    #[test]
    fn index_url_repository_then_origin_then_refusal() {
        let mk = |repo: Option<&str>, origin: Option<&str>| PackageOutput {
            tarball: PathBuf::from("crypto-0.2.1.tar.gz"),
            size: 1,
            sha256: "00".to_string(),
            name: "crypto".to_string(),
            version: "0.2.1".to_string(),
            repository: repo.map(str::to_string),
            origin_repo: origin.map(str::to_string),
            levels: None,
        };
        // monorepo (bare repo → org-relative under loft-lang)
        assert_eq!(
            release_url(&mk(Some("loft-libs-core"), None)).as_deref(),
            Some(
                "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz"
            )
        );
        // explicit owner/repo
        assert_eq!(
            release_url(&mk(Some("acme/widgets"), None)).as_deref(),
            Some(
                "https://github.com/acme/widgets/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz"
            )
        );
        // no `repository` — the origin remote answers, and it answers the same
        // url `loft publish` emits from the same remote.
        assert_eq!(
            release_url(&mk(None, Some("loft-lang/loft-libs-world"))).as_deref(),
            Some(
                "https://github.com/loft-lang/loft-libs-world/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz"
            )
        );
        // `repository` outranks the remote (a package released from elsewhere).
        assert_eq!(
            release_url(&mk(Some("acme/widgets"), Some("loft-lang/loft-libs-world"))).as_deref(),
            Some(
                "https://github.com/acme/widgets/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz"
            )
        );
        // neither: a refusal, and the summary says so instead of printing a url.
        assert_eq!(release_url(&mk(None, None)), None);
        let mut buf = Vec::new();
        print_summary(&mk(None, None), &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains("\"url\""), "{text}");
        assert!(text.contains("Cannot name the release url"), "{text}");
    }

    #[test]
    fn excludes_dotgit_and_target() {
        let dir = tmpdir("excludes_dotgit_and_target");
        let pkg = dir.join("with_excludes");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"with_excludes\"\nversion = \"0.1.0\"\n",
        );
        write(
            &pkg.join("src").join("with_excludes.loft"),
            "// real source\n",
        );
        write(&pkg.join(".git").join("HEAD"), "ref: refs/heads/main\n");
        write(&pkg.join("target").join("debug").join("x"), "binary\n");

        let out = package_create(&pkg, None).expect("package_create");

        // Re-open and walk: confirm .git and target are absent.
        let tar_gz = fs::File::open(&out.tarball).unwrap();
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut ar = tar::Archive::new(dec);
        let mut paths: Vec<String> = Vec::new();
        for entry in ar.entries().unwrap() {
            let entry = entry.unwrap();
            paths.push(entry.path().unwrap().to_string_lossy().into_owned());
        }
        assert!(
            paths.iter().any(|p| p.ends_with("loft.toml")),
            "loft.toml missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains("src/with_excludes.loft")),
            "src missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".git")),
            ".git leaked: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("target/")),
            "target leaked: {paths:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn excludes_gitignored_untracked_files() {
        // A tarball built from a DIRTY tree (gitignored scratch files present —
        // e.g. `tests/_tmp_*.bin` left by `loft test`) must match one built from
        // a clean clone, or the registry's gate-3 reproducible-build check fails.
        // Regression for the hex_world 0.1.x mismatch.
        let dir = tmpdir("excludes_gitignored_untracked_files");
        let pkg = dir.join("ignlib");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"ignlib\"\nversion = \"0.1.0\"\n",
        );
        write(&pkg.join("src").join("ignlib.loft"), "// real source\n");
        write(&pkg.join(".gitignore"), "tests/_tmp_*.bin\n");
        write(&pkg.join("tests").join("_tmp_scratch.bin"), "junk\n");

        // Needs a git repo so git can resolve the ignore rules.  If git is
        // unavailable the fix degrades to a no-op, so skip rather than fail.
        let git_ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&pkg)
            .args(["init", "-q"])
            .status()
            .is_ok_and(|s| s.success());
        if !git_ok {
            let _ = fs::remove_dir_all(&dir);
            return;
        }

        let out = package_create(&pkg, None).expect("package_create");
        let tar_gz = fs::File::open(&out.tarball).unwrap();
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut ar = tar::Archive::new(dec);
        let mut paths: Vec<String> = Vec::new();
        for entry in ar.entries().unwrap() {
            paths.push(
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        assert!(
            paths.iter().any(|p| p.contains("src/ignlib.loft")),
            "src missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("_tmp_scratch.bin")),
            "gitignored scratch file leaked into the package: {paths:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_is_deterministic_across_runs() {
        let dir = tmpdir("sha256_is_deterministic_across_runs");
        let pkg = dir.join("det");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"det\"\nversion = \"0.1.0\"\n",
        );
        write(&pkg.join("src").join("det.loft"), "// content\n");

        let a = package_create(&pkg, None).unwrap();
        // Remove the previous tarball so the second run rebuilds.
        fs::remove_file(&a.tarball).unwrap();
        let b = package_create(&pkg, None).unwrap();

        // Tarballs include mtime in headers, so byte-for-byte equality
        // requires preserving on-disk mtimes — which they are, because
        // we didn't touch the source files between runs.  This is the
        // contract: same inputs → same hash, the assertion in the
        // PKG_REGISTRY.md schema.
        assert_eq!(
            a.sha256, b.sha256,
            "sha256 drifted: {} vs {}",
            a.sha256, b.sha256
        );
        assert_eq!(a.size, b.size);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fails_when_manifest_missing() {
        let dir = tmpdir("fails_when_manifest_missing");
        let pkg = dir.join("empty");
        fs::create_dir_all(&pkg).unwrap();
        let err = package_create(&pkg, None).expect_err("should fail");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let _ = fs::remove_dir_all(&dir);
    }

    /// loft#667 — a local `loft install <dir>` must carry EXACTLY what the published
    /// tarball carries.
    ///
    /// The two used to answer "what does a package consist of" separately, and drifted
    /// twice: `native/` (a local install of an FFI library lost its `n_*` symbols) and then
    /// `wasm/` + the `[wasm.bridge] host_js` file (a local install of a browser library lost
    /// its bridge, and because `~/.loft/lib` is searched before the registry cache, the
    /// incomplete copy shadowed a complete registry one).  Both now read `is_excluded_entry`,
    /// so this diffs the two trees rather than trusting that they agree.
    #[test]
    fn local_install_matches_the_tarball() {
        let dir = tmpdir("install_parity");
        let pkg = dir.join("web");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"web\"\nversion = \"0.3.2\"\n\
             [library]\nentry = \"src/web.loft\"\n\
             [wasm.bridge]\ncrate = \"web-wasm\"\nhost_js = \"wasm/host.js\"\n",
        );
        write(
            &pkg.join("src/web.loft"),
            "pub fn ws_open(url: text) -> integer { return 1; }\n",
        );
        write(
            &pkg.join("tests/web_test.loft"),
            "fn test_ws() { assert(true, \"ok\"); }\n",
        );
        // The two directories the whitelist forgot, plus a nested file in each.
        write(
            &pkg.join("native/Cargo.toml"),
            "[package]\nname = \"web-native\"\n",
        );
        write(&pkg.join("native/src/lib.rs"), "// ffi\n");
        write(
            &pkg.join("wasm/Cargo.toml"),
            "[package]\nname = \"web-wasm\"\n",
        );
        write(&pkg.join("wasm/src/lib.rs"), "// bridge\n");
        write(&pkg.join("wasm/host.js"), "// host shim\n");
        write(&pkg.join("README.md"), "# web\n");
        // Excluded on both paths: build output, a nested build dir, a stale artefact.
        write(&pkg.join("target/debug/junk.bin"), "x\n");
        write(&pkg.join("native/target/debug/junk.bin"), "x\n");
        write(&pkg.join("node_modules/dep/index.js"), "x\n");
        write(&pkg.join("web-0.3.1.tar.gz"), "stale\n");

        // The install path.
        let installed = dir.join("installed");
        copy_package_tree(&pkg, &installed).expect("copy_package_tree");
        let mut local: Vec<String> = Vec::new();
        collect_rel(&installed, &installed, &mut local);
        local.sort();

        // The tarball path — entry names with the `<pkg>-<version>/` prefix stripped.
        let out = package_create(&pkg, Some(&dir)).expect("package_create");
        let f = fs::File::open(&out.tarball).expect("open tarball");
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(f));
        let mut packaged: Vec<String> = archive
            .entries()
            .expect("entries")
            .filter_map(Result::ok)
            .filter(|e| e.header().entry_type().is_file())
            .filter_map(|e| {
                let p = e.path().ok()?.to_path_buf();
                Some(crate::file_access::portable(
                    p.strip_prefix("web-0.3.2").unwrap_or(&p),
                ))
            })
            .collect();
        packaged.sort();
        // The tarball run wrote its own output into `dir`, not into `pkg`, so nothing new
        // appeared under the package between the two walks.
        assert_eq!(
            local, packaged,
            "a local install and the tarball must carry the same files"
        );

        // Non-vacuous: the parity assert would also pass if BOTH dropped the bridge.
        for want in [
            "wasm/host.js",
            "wasm/src/lib.rs",
            "native/src/lib.rs",
            "src/web.loft",
        ] {
            assert!(
                local.iter().any(|f| f == want),
                "the local install must carry `{want}`, got {local:?}"
            );
        }
        for unwanted in [
            "target/debug/junk.bin",
            "native/target/debug/junk.bin",
            "web-0.3.1.tar.gz",
        ] {
            assert!(
                !local.iter().any(|f| f == unwanted),
                "the local install must NOT carry `{unwanted}`"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Relative paths of every file under `dir`, `/`-separated.
    fn collect_rel(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_rel(root, &p, out);
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.push(crate::file_access::portable(rel));
            }
        }
    }
}
