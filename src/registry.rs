// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! Package registry — parse registry files, resolve versions, classify installed packages.

use std::path::{Path, PathBuf};

/// Default registry URL — used when no `source:` header or env var is set.
const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/loft-lang/registry/main/registry.txt";

/// A single entry from the registry file.
#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    pub url: String,
    /// `None` = active; `Some("yanked:reason")` or `Some("deprecated:reason")`.
    pub status: Option<String>,
}

impl RegistryEntry {
    pub fn is_yanked(&self) -> bool {
        self.status.as_deref().unwrap_or("").starts_with("yanked")
    }

    pub fn is_deprecated(&self) -> bool {
        self.status
            .as_deref()
            .unwrap_or("")
            .starts_with("deprecated")
    }

    pub fn is_active(&self) -> bool {
        self.status.is_none()
    }

    /// The part after the `:` in a status field (e.g. `"CVE-2026-001"` from `"yanked:CVE-2026-001"`).
    pub fn status_slug(&self) -> &str {
        match &self.status {
            Some(s) => s.split_once(':').map_or("", |(_, slug)| slug),
            None => "",
        }
    }
}

/// Result of classifying an installed package against the registry.
#[derive(Debug)]
pub enum PackageStatus<'a> {
    /// Installed version is yanked — must update.
    Yanked { entry: &'a RegistryEntry },
    /// Installed version is deprecated — should update.
    Deprecated {
        entry: &'a RegistryEntry,
        latest: Option<&'a RegistryEntry>,
    },
    /// A newer active version exists in the registry.
    Outdated { latest: &'a RegistryEntry },
    /// Installed version is the highest active version.
    Current,
    /// Package name not found in registry at all.
    Unknown,
}

/// Parse a registry file.  Returns all entries (including yanked/deprecated)
/// and the `source:` URL extracted from the header comment, if present.
pub fn read_registry(path: &str) -> (Vec<RegistryEntry>, Option<String>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (Vec::new(), None);
    };
    parse_registry(&content)
}

/// Parse registry content from a string.
pub fn parse_registry(content: &str) -> (Vec<RegistryEntry>, Option<String>) {
    let mut entries = Vec::new();
    let mut source_url = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix('#') {
            // Check for source: directive (only first one).
            if source_url.is_none()
                && let Some(url) = comment.trim().strip_prefix("source:")
            {
                source_url = Some(url.trim().to_string());
            }
            continue;
        }
        // Data line: <name> <version> <url> [status]
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 3 {
            continue; // Malformed line — skip.
        }
        entries.push(RegistryEntry {
            name: fields[0].to_string(),
            version: fields[1].to_string(),
            url: fields[2].to_string(),
            status: fields.get(3).map(|s| (*s).to_string()),
        });
    }
    (entries, source_url)
}

/// Find the local registry file path.
/// Checks `LOFT_REGISTRY` env var, then `~/.loft/registry.txt`.
pub fn registry_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LOFT_REGISTRY") {
        let path = PathBuf::from(&p);
        if path.exists() {
            return Some(path);
        }
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let path = Path::new(&home).join(".loft").join("registry.txt");
    if path.exists() { Some(path) } else { None }
}

/// Determine the registry source URL for sync.
/// Priority: `LOFT_REGISTRY_URL` env var → `source:` header from file → compiled-in default.
pub fn source_url(file_source: Option<&str>) -> String {
    if let Ok(url) = std::env::var("LOFT_REGISTRY_URL")
        && !url.is_empty()
    {
        return url;
    }
    if let Some(url) = file_source
        && !url.is_empty()
    {
        return url.to_string();
    }
    DEFAULT_REGISTRY_URL.to_string()
}

/// Parse a semver string into a comparable tuple.
fn parse_version(s: &str) -> (u32, u32, u32) {
    let mut parts = s.splitn(3, '.');
    let major = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Find the best matching entry for install.
/// `version=None` → highest semver active entry (skips yanked and deprecated if active exists).
/// `version=Some` → exact match (any status, but warns for yanked).
pub fn find_package<'a>(
    entries: &'a [RegistryEntry],
    name: &str,
    version: Option<&str>,
) -> Option<&'a RegistryEntry> {
    if let Some(ver) = version {
        // Exact version match — first entry wins (top-to-bottom).
        return entries.iter().find(|e| e.name == name && e.version == ver);
    }
    // Latest: highest semver among active entries.
    let mut best: Option<&'a RegistryEntry> = None;
    for entry in entries.iter().filter(|e| e.name == name) {
        if entry.is_yanked() {
            continue;
        }
        // Prefer active over deprecated.
        if let Some(current) = best {
            if current.is_active() && entry.is_deprecated() {
                continue;
            }
            if parse_version(&entry.version) > parse_version(&current.version) {
                best = Some(entry);
            }
        } else {
            best = Some(entry);
        }
    }
    best
}

/// Collect all unique package names from the registry.
pub fn package_names(entries: &[RegistryEntry]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for e in entries {
        if !names.contains(&e.name) {
            names.push(e.name.clone());
        }
    }
    names.sort();
    names
}

/// Find all versions for a given package name.
pub fn package_versions<'a>(entries: &'a [RegistryEntry], name: &str) -> Vec<&'a RegistryEntry> {
    entries.iter().filter(|e| e.name == name).collect()
}

/// Scan a library directory for installed packages.
/// Returns `(name, version)` for each subdirectory containing a readable `loft.toml`.
pub fn installed_packages(lib_dir: &Path) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(lib_dir) else {
        return result;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("loft.toml");
        if !manifest_path.exists() {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // Read version from loft.toml.
        let version = match std::fs::read_to_string(&manifest_path) {
            Ok(content) => extract_version(&content),
            Err(_) => String::new(),
        };
        result.push((name, version));
    }
    result.sort();
    result
}

/// Extract version string from loft.toml content.
fn extract_version(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let val = rest.trim().trim_matches('"');
                return val.to_string();
            }
        }
    }
    String::new()
}

/// Classify an installed package against the registry.
pub fn classify<'a>(entries: &'a [RegistryEntry], name: &str, version: &str) -> PackageStatus<'a> {
    let pkg_entries: Vec<&RegistryEntry> = entries.iter().filter(|e| e.name == name).collect();
    if pkg_entries.is_empty() {
        return PackageStatus::Unknown;
    }
    // Check installed version's status.
    if let Some(installed_entry) = pkg_entries.iter().find(|e| e.version == version) {
        if installed_entry.is_yanked() {
            return PackageStatus::Yanked {
                entry: installed_entry,
            };
        }
        if installed_entry.is_deprecated() {
            let latest = find_package(entries, name, None);
            return PackageStatus::Deprecated {
                entry: installed_entry,
                latest,
            };
        }
    }
    // Check if a newer active version exists.
    if let Some(latest) = find_package(entries, name, None)
        && parse_version(&latest.version) > parse_version(version)
    {
        return PackageStatus::Outdated { latest };
    }
    PackageStatus::Current
}

/// Return the default library directory (`~/.loft/lib/`).
pub fn lib_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".loft").join("lib")
}

/// Return the default registry file path (`~/.loft/registry.txt`).
pub fn default_registry_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".loft").join("registry.txt")
}

/// Download a URL to a local file path.  Returns `Err` with a human-readable message on failure.
///
/// # Errors
///
/// Returns an error string if the HTTP request fails, the destination cannot
/// be created, or the response body cannot be copied to disk.
#[cfg(feature = "registry")]
pub fn download_file(url: &str, dst: &Path) -> Result<(), String> {
    let resp = crate::tls::agent_builder()
        .build()
        .get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let mut reader = resp.into_reader();
    let mut file =
        std::fs::File::create(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("write failed: {e}"))?;
    Ok(())
}

/// Download and extract a package zip.  Returns the path to the package root
/// (directory containing `loft.toml`).
///
/// # Errors
///
/// Returns an error string if the download fails, the temp directory cannot
/// be created, the zip is malformed, extraction fails, or no `loft.toml`
/// is found in the extracted contents.
#[cfg(feature = "registry")]
pub fn download_and_extract(entry: &RegistryEntry, tmp_base: &Path) -> Result<PathBuf, String> {
    // Download zip to temp file.
    let zip_path = tmp_base.join(format!("{}-{}.zip", entry.name, entry.version));
    download_file(&entry.url, &zip_path)?;

    // Extract zip.
    let extract_dir = tmp_base.join(format!("{}-{}", entry.name, entry.version));
    std::fs::create_dir_all(&extract_dir).map_err(|e| format!("cannot create temp dir: {e}"))?;

    let file = std::fs::File::open(&zip_path).map_err(|e| format!("cannot open zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("invalid zip archive: {e}"))?;
    archive
        .extract(&extract_dir)
        .map_err(|e| format!("zip extraction failed: {e}"))?;

    // Find package root — directory containing loft.toml.
    find_package_root(&extract_dir).ok_or_else(|| {
        "could not find package root in downloaded zip.\n  \
             Expected loft.toml or src/ directory inside the archive."
            .to_string()
    })
}

/// Recursively find a directory containing `loft.toml` (depth-first, first match).
/// Falls back to a directory containing `src/` if no `loft.toml` is found.
#[cfg(feature = "registry")]
fn find_package_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("loft.toml").exists() {
        return Some(dir.to_path_buf());
    }
    // Check immediate children first.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("loft.toml").exists() {
                return Some(path);
            }
        }
    }
    // Fallback: check for src/ at root.
    if dir.join("src").is_dir() {
        return Some(dir.to_path_buf());
    }
    None
}

/// Format the registry staleness warning if applicable.
pub fn staleness_warning(registry_path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(registry_path).ok()?;
    let modified = metadata.modified().ok()?;
    let age = std::time::SystemTime::now().duration_since(modified).ok()?;
    let days = age.as_secs() / 86400;
    if days >= 7 {
        Some(format!(
            "warning: registry was last synced {days} days ago.\n  \
             Run 'loft registry sync' to get the latest security information."
        ))
    } else {
        None
    }
}

/// Validate downloaded registry content.
/// Returns `Ok(())` if the content is valid, `Err` with a reason otherwise.
///
/// # Errors
///
/// Returns an error string if the content is empty or contains no parseable
/// registry entries.
pub fn validate_registry_content(content: &str) -> Result<(), String> {
    if content.is_empty() {
        return Err("downloaded registry is empty".to_string());
    }
    let data_lines = content
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .count();
    if data_lines == 0 {
        return Err("downloaded registry contains no package entries".to_string());
    }
    // Basic format check: each data line should have at least 3 fields.
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = t.split_whitespace().collect();
        if fields.len() < 3 {
            return Err(format!("malformed registry line: {t}"));
        }
    }
    Ok(())
}

/// Count packages and versions in a registry.
pub fn registry_stats(entries: &[RegistryEntry]) -> (usize, usize) {
    let names = package_names(entries);
    (names.len(), entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_REGISTRY: &str = "\
# source: https://example.com/registry.txt
# Loft package registry

graphics 0.1.0 https://example.com/graphics-0.1.0.zip yanked:CVE-2026-001
graphics 0.2.0 https://example.com/graphics-0.2.0.zip
opengl   0.1.0 https://example.com/opengl-0.1.0.zip   deprecated:use-graphics
math     1.0.0 https://example.com/math-1.0.0.zip
math     1.1.0 https://example.com/math-1.1.0.zip
";

    #[test]
    fn parse_registry_entries() {
        let (entries, source) = parse_registry(SAMPLE_REGISTRY);
        assert_eq!(entries.len(), 5);
        assert_eq!(source, Some("https://example.com/registry.txt".to_string()));
        assert_eq!(entries[0].name, "graphics");
        assert_eq!(entries[0].version, "0.1.0");
        assert!(entries[0].is_yanked());
        assert_eq!(entries[0].status_slug(), "CVE-2026-001");
        assert!(entries[1].is_active());
        assert!(entries[2].is_deprecated());
        assert_eq!(entries[2].status_slug(), "use-graphics");
    }

    #[test]
    fn find_latest_skips_yanked() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        let best = find_package(&entries, "graphics", None).unwrap();
        assert_eq!(best.version, "0.2.0");
        assert!(best.is_active());
    }

    #[test]
    fn find_exact_version() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        let entry = find_package(&entries, "graphics", Some("0.1.0")).unwrap();
        assert_eq!(entry.version, "0.1.0");
        assert!(entry.is_yanked());
    }

    #[test]
    fn find_latest_math() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        let best = find_package(&entries, "math", None).unwrap();
        assert_eq!(best.version, "1.1.0");
    }

    #[test]
    fn find_nonexistent() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        assert!(find_package(&entries, "nonexistent", None).is_none());
    }

    #[test]
    fn classify_yanked() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        assert!(matches!(
            classify(&entries, "graphics", "0.1.0"),
            PackageStatus::Yanked { .. }
        ));
    }

    #[test]
    fn classify_deprecated() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        assert!(matches!(
            classify(&entries, "opengl", "0.1.0"),
            PackageStatus::Deprecated { .. }
        ));
    }

    #[test]
    fn classify_outdated() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        assert!(matches!(
            classify(&entries, "math", "1.0.0"),
            PackageStatus::Outdated { .. }
        ));
    }

    #[test]
    fn classify_current() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        assert!(matches!(
            classify(&entries, "math", "1.1.0"),
            PackageStatus::Current
        ));
    }

    #[test]
    fn classify_unknown() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        assert!(matches!(
            classify(&entries, "unknown", "0.1.0"),
            PackageStatus::Unknown
        ));
    }

    #[test]
    fn version_ordering() {
        assert!(parse_version("1.1.0") > parse_version("1.0.0"));
        assert!(parse_version("0.2.0") > parse_version("0.1.0"));
        assert!(parse_version("1.0.0") > parse_version("0.99.99"));
        assert_eq!(parse_version("1.0.0"), parse_version("1.0.0"));
    }

    #[test]
    fn source_url_priority() {
        // With no env vars set, file source takes priority over default.
        let url = source_url(Some("https://custom.example.com/registry.txt"));
        assert_eq!(url, "https://custom.example.com/registry.txt");

        // Empty file source falls back to default.
        let url = source_url(None);
        assert_eq!(url, DEFAULT_REGISTRY_URL);
    }

    #[test]
    fn validate_good_registry() {
        assert!(validate_registry_content(SAMPLE_REGISTRY).is_ok());
    }

    #[test]
    fn validate_empty_registry() {
        assert!(validate_registry_content("").is_err());
        assert!(validate_registry_content("# only comments\n").is_err());
    }

    #[test]
    fn validate_malformed_line() {
        assert!(validate_registry_content("bad line\n").is_err());
    }

    #[test]
    fn registry_stats_count() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        let (pkgs, versions) = registry_stats(&entries);
        assert_eq!(pkgs, 3); // graphics, opengl, math
        assert_eq!(versions, 5);
    }

    #[test]
    fn extract_version_from_toml() {
        let toml = r#"
name = "test"
version = "0.3.0"
loft = ">=0.8.0"
"#;
        assert_eq!(extract_version(toml), "0.3.0");
    }

    #[test]
    fn package_names_sorted() {
        let (entries, _) = parse_registry(SAMPLE_REGISTRY);
        let names = package_names(&entries);
        assert_eq!(names, vec!["graphics", "math", "opengl"]);
    }
}
