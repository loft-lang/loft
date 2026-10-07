// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! What a package consists of — the ONE answer, for everyone who needs it.
//!
//! Two commands copy a package's files: `loft package` bundles the tarball, and
//! `loft install <dir>` copies a local tree into `~/.loft/lib/<name>/`.  They used to
//! answer "which files?" separately — the tarball by exclusion, the install by a
//! whitelist of `loft.toml` + `src/*.loft` + `tests/` + `native/` — and the two answers
//! disagreed twice.  First `native/`: a local install of an FFI library silently dropped
//! it, so the consumer got undefined `n_*` symbols at link time.  Then `wasm/`: a local
//! install of a `[wasm.bridge]` library dropped the bridge, and because `~/.loft/lib` is
//! searched BEFORE the registry cache, the incomplete copy shadowed a complete registry
//! one — every `--html` build against that library then failed to link, with an error
//! naming the library rather than the install that broke it (loft#667).
//!
//! A whitelist re-derives the answer, so it can only ever lag by one directory.  Both
//! commands now read [`is_excluded_entry`].
//!
//! This module is deliberately OUTSIDE `package` (which is `registry`-gated and pulls in
//! tar/flate2/sha2): `loft install <dir>` works in a `--no-default-features` build, so the
//! rule it depends on must compile there too.

use crate::file_access;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// Directory names never included in a package, at any depth (`native/target/` nests).
pub(crate) const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "target",
    ".loft",
    "node_modules",
    ".vscode",
    ".idea",
];

/// Case-insensitive ASCII suffix check.  `str::ends_with` is case-sensitive and
/// `to_ascii_lowercase` allocates; this avoids both.
fn has_suffix_ci(s: &str, suffix: &str) -> bool {
    let sb = s.as_bytes();
    let tb = suffix.as_bytes();
    sb.len() >= tb.len() && sb[sb.len() - tb.len()..].eq_ignore_ascii_case(tb)
}

/// File names excluded recursively — tar artefacts, so a re-run from the same directory
/// bundles neither the archive being written nor a stale one from last release.
fn is_excluded_file(name: &str) -> bool {
    has_suffix_ci(name, ".tar.gz") || has_suffix_ci(name, ".tar")
}

/// Paths git ignores, relative to `root`.  Keeps a package built from a dirty working
/// tree byte-identical to one built from a clean clone (the reproducible-build gate).
/// Returns empty when git is absent or `root` is not a repository — i.e. behaves as if
/// nothing were ignored, never as if everything were.
pub(crate) fn git_ignored_set(root: &Path) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ])
        .output()
    else {
        return set; // git not installed → behave as before
    };
    if !out.status.success() {
        return set; // not inside a git repo → behave as before
    }
    for p in out.stdout.split(|&b| b == 0) {
        if !p.is_empty() {
            let s = String::from_utf8_lossy(p);
            set.insert(PathBuf::from(s.trim_end_matches('/')));
        }
    }
    set
}

/// Is this directory entry excluded from a package?  **The single answer** — see the
/// module docs for the two directories a second answer lost.
pub(crate) fn is_excluded_entry(
    root: &Path,
    path: &Path,
    name: &str,
    is_dir: bool,
    ignored: &HashSet<PathBuf>,
) -> bool {
    if path
        .strip_prefix(root)
        .is_ok_and(|rel| ignored.contains(rel))
    {
        return true;
    }
    if is_dir && EXCLUDED_DIRS.contains(&name) {
        return true;
    }
    !is_dir && is_excluded_file(name)
}

/// Copy a package tree the way `loft package` bundles it — same include rule, so a local
/// `loft install <dir>` carries exactly what the published tarball carries.  Returns the
/// number of files copied.
///
/// # Errors
/// Propagates any I/O error from reading `root` or writing under `dst`.
pub fn copy_package_tree(root: &Path, dst: &Path) -> io::Result<usize> {
    let ignored = git_ignored_set(root);
    copy_tree_inner(root, root, dst, &ignored)
}

fn copy_tree_inner(
    root: &Path,
    dir: &Path,
    dst: &Path,
    ignored: &HashSet<PathBuf>,
) -> io::Result<usize> {
    file_access::create_dir_all(dst)?;
    let mut copied = 0;
    for entry in file_access::read_dir(dir)? {
        let Some(name) = entry.os_name() else {
            continue;
        };
        // Spelled under `dir` as given, so `is_excluded_entry` strips `root` as it is spelled.
        let path = dir.join(&name);
        let name_str = name.to_string_lossy();
        let file_type = file_access::symlink_metadata(&path)?.file_type();
        if is_excluded_entry(root, &path, name_str.as_ref(), file_type.is_dir(), ignored) {
            continue;
        }
        if file_type.is_dir() {
            copied += copy_tree_inner(root, &path, &dst.join(&name), ignored)?;
        } else if file_type.is_file() {
            file_access::copy(&path, dst.join(&name))?;
            copied += 1;
        }
    }
    Ok(copied)
}
