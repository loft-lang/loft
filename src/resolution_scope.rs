// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! Which declaration governs a program's dependency versions — @PLN143 arc B.
//!
//! A `use <pkg>` picks a registry version, and exactly one file decides which: the
//! project's `loft.lock`, a `<script>.loft.lock` beside the script, or — where neither
//! exists — nothing at all, which means *the newest release, re-decided every run*.
//!
//! | Scope | Detected by | Governs |
//! |---|---|---|
//! | [`ResolutionScope::PinnedScript`] | `<script>.loft.lock` beside the script | that sidecar |
//! | [`ResolutionScope::Package`] | nearest ancestor `loft.toml` | that root's `loft.lock` |
//! | [`ResolutionScope::Bare`] | neither | nothing — newest release, every run |
//!
//! **The cwd plays no part.** It used to, and that was the defect: the same script run
//! from two directories resolved two different ways, and the file that pinned it was
//! written by an earlier RUN rather than by anyone's decision.
//!
//! The scope is a property of the PROGRAM, so it is answered once, here. Before this the
//! policy WAS the parser's probe order, and three probes each re-derived their own
//! lockfile path independently — a disagreement between them is silent: a different
//! version loads and nothing errors.
//!
//! Not to be confused with [`crate::resolution`], which is the LSP's name-resolution
//! index. This module resolves *versions*, not identifiers.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::path::{Path, PathBuf};

/// The declaration in force for a program's registry dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionScope {
    /// A package: the nearest ancestor directory holding a `loft.toml`. Its
    /// `loft.lock` governs, and `loft install` records there.
    Package(PathBuf),
    /// A single script pinned by `loft pin <script>`: the `<script>.loft.lock`
    /// beside it. Takes precedence over an enclosing package, because it is the
    /// declaration nearest the thing it governs.
    PinnedScript(PathBuf),
    /// Neither — nothing is declared, so `use <pkg>` means the newest release,
    /// re-decided on every run.
    Bare,
}

impl ResolutionScope {
    /// The lockfile that decides versions for this program, or `None` in [`Self::Bare`]
    /// scope, where nothing does.
    #[must_use]
    pub fn governing_lock(&self) -> Option<PathBuf> {
        match self {
            Self::Package(root) => Some(root.join("loft.lock")),
            Self::PinnedScript(sidecar) => Some(sidecar.clone()),
            Self::Bare => None,
        }
    }

    /// The version this scope's lockfile pins for `pkg`, if it names one.
    ///
    /// The declaration in force, read as a value rather than as a resolution: a lock
    /// entry is an EXACT version, so it answers both "which file do I load" and — when
    /// that file is not in the cache yet — "which version must be installed". Those were
    /// two different questions with two different answers before @PLN143: the load
    /// honoured the pin and the install ignored it, so a fresh box quietly ran a
    /// different version of the program than the machine that pinned it.
    ///
    /// `None` in `Bare` scope (nothing is declared), for a lock that cannot be read, and
    /// for a lock that does not name `pkg`.
    #[must_use]
    pub fn pinned_version(&self, pkg: &str) -> Option<String> {
        let lock_path = self.governing_lock()?;
        let lock = crate::lockfile::read_lockfile(&lock_path).ok()??;
        lock.packages
            .into_iter()
            .find(|p| p.name == pkg)
            .map(|p| p.version)
    }

    /// Where an auto-install may record what it resolved — `None` when it may record
    /// nothing.
    ///
    /// Only a package has a place to record: the lock beside the manifest that declares
    /// the dependency, which is the same file `loft install` writes. A pinned script
    /// already carries its declaration, and a bare script has none — writing one on the
    /// program's behalf is what made a first run decide every run after it, which is the
    /// invariant this plan exists for.
    ///
    /// `in_registry_cache` is the one case that overrides the scope: a transitive dep
    /// discovered while parsing a file that already lives under `~/.loft/registry/` has
    /// no consumer project — the only `loft.toml` above it is the cached dependency's
    /// own, so recording there would write into the immutable cache.
    #[must_use]
    pub fn lock_write_target(&self, in_registry_cache: bool) -> Option<PathBuf> {
        if in_registry_cache {
            return None;
        }
        match self {
            Self::Package(root) => Some(root.join("loft.lock")),
            Self::PinnedScript(_) | Self::Bare => None,
        }
    }
}

/// Which declaration governs the program in `script_path`.
///
/// `script_path` may be empty (the REPL, `-e`), in which case the walk-up starts at the
/// current directory — a REPL inside a package still resolves that package's versions.
#[must_use]
pub fn resolution_scope(script_path: &str) -> ResolutionScope {
    if !script_path.is_empty() {
        let sidecar = PathBuf::from(format!("{script_path}.lock"));
        if sidecar.exists() {
            return ResolutionScope::PinnedScript(sidecar);
        }
    }
    match project_root(script_path) {
        Some(root) => ResolutionScope::Package(root),
        None => ResolutionScope::Bare,
    }
}

/// The nearest ancestor directory of `script_path` that holds a `loft.toml`, or `None`
/// when the walk reaches the filesystem root without finding one.
///
/// `script_path` may name a file or a directory; an empty path starts at the current
/// directory.
///
/// Not gated on the `registry` feature, though its callers mostly are: it walks up
/// looking for a `loft.toml` and asks the network nothing. Gating it there put a plain
/// path walk out of reach of a `--no-default-features` build, which is the shape loft's
/// own wasm runtime rlib is compiled in (the gated-by-association mistake of loft#967).
#[must_use]
pub fn project_root(script_path: &str) -> Option<PathBuf> {
    let p = Path::new(script_path);
    let start_dir = if script_path.is_empty() {
        std::env::current_dir().ok()?
    } else if p.is_dir() {
        p.to_path_buf()
    } else {
        let parent = p.parent()?;
        if parent.as_os_str().is_empty() {
            std::env::current_dir().ok()?
        } else {
            parent.to_path_buf()
        }
    };
    project_root_from(&start_dir)
}

/// [`project_root`] from a directory that is already in hand.
///
/// Canonicalized first, so the walk does not terminate prematurely on a relative `./`
/// prefix that loops on itself — in the PLAIN spelling (`file_access::plain_canonical`),
/// because the root is what a source position's absolute `file` is stripped against, and a
/// verbatim `\\?\D:\…` root never prefix-matches the plain path the driver records.
#[must_use]
pub fn project_root_from(start: &Path) -> Option<PathBuf> {
    let abs = crate::file_access::plain_canonical(start);
    let mut cur = abs.as_path();
    loop {
        if cur.join("loft.toml").exists() {
            return Some(cur.to_path_buf());
        }
        let parent = cur.parent()?;
        if parent == cur {
            return None;
        }
        cur = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("loft_scope_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, body).expect("write");
    }

    /// The scope table, asserted row by row: a script inside a package, a script with a
    /// sidecar pin, and a script with neither.
    #[test]
    fn the_scope_table() {
        let root = tmp("table");
        write(&root.join("pkg/loft.toml"), "[package]\nname = \"p\"\n");
        write(&root.join("pkg/src/s.loft"), "fn main() {}\n");
        write(&root.join("bare/s.loft"), "fn main() {}\n");
        write(&root.join("pinned/s.loft"), "fn main() {}\n");
        write(&root.join("pinned/s.loft.lock"), "schema_version = 1\n");

        let pkg_script = root.join("pkg/src/s.loft");
        assert_eq!(
            resolution_scope(&pkg_script.to_string_lossy()),
            ResolutionScope::Package(crate::file_access::plain_canonical(&root.join("pkg")))
        );
        assert_eq!(
            resolution_scope(&root.join("bare/s.loft").to_string_lossy()),
            ResolutionScope::Bare
        );
        assert_eq!(
            resolution_scope(&root.join("pinned/s.loft").to_string_lossy()),
            ResolutionScope::PinnedScript(root.join("pinned/s.loft.lock"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A sidecar beside the script wins over an enclosing package: it is the declaration
    /// nearest the thing it governs, and `loft pin <script>` wrote it for THIS script.
    /// (The probe order said the same before the scope existed; this is where it is now
    /// decided, and the two must not disagree.)
    #[test]
    fn a_sidecar_outranks_an_enclosing_package() {
        let root = tmp("both");
        write(&root.join("loft.toml"), "[package]\nname = \"p\"\n");
        write(&root.join("src/s.loft"), "fn main() {}\n");
        write(&root.join("src/s.loft.lock"), "schema_version = 1\n");
        assert_eq!(
            resolution_scope(&root.join("src/s.loft").to_string_lossy()),
            ResolutionScope::PinnedScript(root.join("src/s.loft.lock"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The cwd is not an input. The same script answers the same scope from anywhere —
    /// which is the whole point of the type: resolution used to read `cwd/loft.lock`, so
    /// where you stood decided which version you got.
    #[test]
    fn the_scope_does_not_depend_on_the_cwd() {
        let root = tmp("cwd");
        write(&root.join("bare/s.loft"), "fn main() {}\n");
        // A lockfile in the directory we stand in must not make this a scope.
        write(&root.join("loft.lock"), "schema_version = 1\n");
        let script = root.join("bare/s.loft");
        assert_eq!(
            resolution_scope(&script.to_string_lossy()),
            ResolutionScope::Bare
        );
        assert_eq!(
            resolution_scope(&script.to_string_lossy()).governing_lock(),
            None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A scope answers what its own lockfile pins — the sidecar for a pinned script, the
    /// root lock for a package, nothing at all for a bare script.
    #[test]
    fn a_scope_answers_the_version_its_lock_pins() {
        let root = tmp("pinned_version");
        let lock = |name: &str, version: &str| {
            format!(
                "schema_version = 1\n\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
                 url = \"http://example.invalid/{name}-{version}.tar.gz\"\n\
                 sha256 = \"00\"\nsource = \"registry\"\n"
            )
        };
        write(&root.join("pkg/loft.toml"), "[package]\nname = \"p\"\n");
        write(&root.join("pkg/loft.lock"), &lock("probepkg", "0.4.0"));
        write(&root.join("pkg/src/s.loft"), "fn main() {}\n");
        write(&root.join("pinned/s.loft"), "fn main() {}\n");
        write(&root.join("pinned/s.loft.lock"), &lock("probepkg", "0.1.0"));
        write(&root.join("bare/s.loft"), "fn main() {}\n");

        let pinned = resolution_scope(&root.join("pinned/s.loft").to_string_lossy());
        assert_eq!(pinned.pinned_version("probepkg").as_deref(), Some("0.1.0"));
        assert_eq!(
            pinned.pinned_version("other"),
            None,
            "a lock that does not name the package pins nothing"
        );

        let package = resolution_scope(&root.join("pkg/src/s.loft").to_string_lossy());
        assert_eq!(package.pinned_version("probepkg").as_deref(), Some("0.4.0"));

        let bare = resolution_scope(&root.join("bare/s.loft").to_string_lossy());
        assert_eq!(
            bare.pinned_version("probepkg"),
            None,
            "nothing is declared, so nothing is pinned — the newest release, every run"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What each scope WRITES, which is the other half of the invariant: a package
    /// records beside its manifest, and a resolution that started inside the registry
    /// cache names no target at all.
    #[test]
    fn only_a_package_is_written_to() {
        let root = PathBuf::from("/proj");
        assert_eq!(
            ResolutionScope::Package(root.clone()).lock_write_target(false),
            Some(root.join("loft.lock"))
        );
        assert_eq!(
            ResolutionScope::Package(root).lock_write_target(true),
            None,
            "a dep resolved inside the registry cache records nowhere"
        );
        assert_eq!(
            ResolutionScope::PinnedScript(PathBuf::from("/s.loft.lock")).lock_write_target(false),
            None
        );
        assert_eq!(ResolutionScope::Bare.lock_write_target(false), None);
    }
}
