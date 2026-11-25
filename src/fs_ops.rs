// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I66 — Bytecode VM / executor: the file-system operations a program reaches

//! The file-system operations a program reaches, on the real file system or, under `host_fs`
//! (the interpreter built for the browser), through the host bridge: one home for the
//! interpreter and the C runtime's host calls, so the two cannot answer a path differently.

use crate::database::Stores;
#[cfg(not(host_fs))]
use std::io::Read as _;

/// Read `path` as UTF-8 text into `buf` — the single home for the read behind
/// `file(p).content()`, called by the interpreter (`State::get_file_text`).
///
/// Leaves `buf` EMPTY when the file cannot be opened or its bytes are not valid
/// UTF-8, and warns on the latter.  Emptiness alone does not tell the caller
/// which happened; the loft-level `content()` reads the file's byte size beside
/// it and answers null when a file that HAS bytes yielded no text (loft#829).
#[cfg(not(host_fs))]
pub fn read_file_text_into(path: &str, buf: &mut String) {
    buf.clear();
    let at = crate::file_access::at(path);
    let Ok(mut f) = crate::file_access::open(&at) else {
        return;
    };
    match f.read_to_string(buf) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            buf.clear();
            let size = crate::file_access::metadata(&at).map_or(0, |m| m.len());
            crate::loft_eprintln!(
                "warning: file({path:?}).content() got non-UTF-8 bytes ({size} bytes in \
                 file) — returning null. Read the bytes exactly with `read_bytes(path)`, \
                 or a field at a time with `f#format = LittleEndian; f#read(n)` (or \
                 `BigEndian`); see the loft-write skill § File I/O."
            );
        }
        Err(_) => buf.clear(),
    }
}

/// Read `path` as UTF-8 text into `buf` through the host bridge.  Leaves `buf`
/// EMPTY when the path is absent or its bytes are not text — the same two
/// outcomes the native reader collapses, so `content()` answers alike on both.
#[cfg(host_fs)]
pub fn read_file_text_into(path: &str, buf: &mut String) {
    buf.clear();
    if let Some(text) = crate::wasm::host_fs_read_text(path) {
        *buf = text;
    }
}

// ── FS-B  File-system dispatch helpers ───────────────────────────────────────
// Under `host_fs` (the wasm-bindgen bundle, and `--html`) each function calls
// the host bridge; otherwise it calls the real filesystem.

// Filesystem-op result codes shared by the interpreter and the native runtime.
// The `default/02_files.loft` wrappers map these to `FileResult` variants via an
// explicit `match`, so the codes are deliberately decoupled from the enum's
// declaration order — reordering the loft enum can never silently remap a code.
const FS_OK: i64 = 0;
const FS_OTHER: i64 = 9;
// Ungated: the `*_at` wrappers answer `NotFound` for a path outside the project
// on every target, so this code is reachable in the wasm build too (loft#708).
const FS_NOT_FOUND: i64 = 1;
#[cfg(not(host_fs))]
const FS_PERMISSION_DENIED: i64 = 2;
#[cfg(not(host_fs))]
const FS_IS_DIRECTORY: i64 = 3;
// loft#1256 — `rmdir` refused because the directory still holds entries.  Its own code
// because it is the one failure a caller can ACT on: empty the directory and retry.  Folding
// it into `FS_OTHER` would leave the only recoverable case indistinguishable from the
// unrecoverable ones.
#[cfg(not(host_fs))]
const FS_NOT_EMPTY: i64 = 4;

/// Classify a filesystem mutation's result into an `FS_*` code.  `dir_is_err`
/// asks for the "target is a directory" failure to be reported as
/// `FS_IS_DIRECTORY` (true for `remove_file`, which refuses directories); it is
/// detected by a portable `fs_is_dir` stat rather than `ErrorKind::IsADirectory`,
/// which only stabilised in Rust 1.83 and this file is recompiled by the user's
/// rustc for `--native`.
#[cfg(not(host_fs))]
fn fs_classify(r: std::io::Result<()>, path: &str, dir_is_err: bool) -> i64 {
    match r {
        Ok(()) => FS_OK,
        Err(e) => {
            // "target is a directory" must be checked BEFORE PermissionDenied: Linux
            // reports `remove_file` on a directory as EISDIR, but macOS/BSD report it as
            // EPERM (→ ErrorKind::PermissionDenied).  Consulting the portable `fs_is_dir`
            // stat first makes a directory target classify as FS_IS_DIRECTORY uniformly,
            // instead of leaking the platform's errno (a real EPERM on a *file* still
            // falls through, because `fs_is_dir` is false for it and for a missing path).
            if dir_is_err && fs_is_dir(path) {
                return FS_IS_DIRECTORY;
            }
            match e.kind() {
                std::io::ErrorKind::NotFound => FS_NOT_FOUND,
                std::io::ErrorKind::PermissionDenied => FS_PERMISSION_DENIED,
                _ => FS_OTHER,
            }
        }
    }
}

/// Remove the EMPTY directory `path`.  Returns an `FS_*` code (mapped to `FileResult` by the
/// `rmdir` wrapper).
///
/// Empty directories only, deliberately: a recursive removal is the sharpest tool a
/// filesystem API has, and it deserves its own decision rather than arriving inside the
/// primitive that closes the create/remove asymmetry.  A caller that wants one writes the
/// walk with `list_dir` and this.
///
/// "Not empty" is detected by STATTING rather than by `ErrorKind::DirectoryNotEmpty`, for the
/// reason the `IsADirectory` comment above gives: that variant stabilised in Rust 1.83, and
/// this file is recompiled by the USER's rustc for `--native`.  The kind is consulted first
/// so a genuine permission error on a non-empty directory keeps its own answer.
pub fn fs_rmdir(path: &str) -> i64 {
    #[cfg(host_fs)]
    {
        // The browser host bridges `mkdir` but has no `rmdir` counterpart, so this refuses
        // rather than reporting a removal that did not happen — `FileResult.Other`, which the
        // enum's own doc already names as the wasm host's catch-all.
        let _ = path;
        FS_OTHER
    }
    #[cfg(not(host_fs))]
    {
        let at = crate::file_access::at(path);
        match crate::file_access::remove_dir(&at) {
            Ok(()) => FS_OK,
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => FS_NOT_FOUND,
                std::io::ErrorKind::PermissionDenied => FS_PERMISSION_DENIED,
                _ if fs_is_dir(path)
                    && crate::file_access::read_dir(&at).is_ok_and(|d| !d.is_empty()) =>
                {
                    FS_NOT_EMPTY
                }
                _ => FS_OTHER,
            },
        }
    }
}

/// Delete file `path`.  Returns an `FS_*` code (mapped to `FileResult` by the
/// `delete` wrapper).  A directory target yields `FS_IS_DIRECTORY`.
pub fn fs_delete(path: &str) -> i64 {
    // The host reports one failure code: a path that is not there is `NotFound`, as the OS
    // error classifies it on the other targets (`@FR-Path-Refuse` keeps `Other` for a refused
    // path, answered before this by `fs_delete_at`).
    #[cfg(host_fs)]
    {
        if crate::wasm::host_fs_delete(path) == 0 {
            FS_OK
        } else if !crate::wasm::host_fs_exists(path) {
            FS_NOT_FOUND
        } else {
            FS_OTHER
        }
    }
    #[cfg(not(host_fs))]
    {
        fs_classify(
            crate::file_access::remove_file(crate::file_access::at(path)),
            path,
            true,
        )
    }
}

/// Rename / move `from` to `to`.  Returns an `FS_*` code.
#[must_use]
pub fn fs_move(from: &str, to: &str) -> i64 {
    // A missing source is `NotFound` on every target, as `fs_delete`'s host arm says.
    #[cfg(host_fs)]
    {
        if crate::wasm::host_fs_move(from, to) == 0 {
            FS_OK
        } else if !crate::wasm::host_fs_exists(from) {
            FS_NOT_FOUND
        } else {
            FS_OTHER
        }
    }
    #[cfg(not(host_fs))]
    {
        fs_classify(
            crate::file_access::rename(crate::file_access::at(from), crate::file_access::at(to)),
            from,
            false,
        )
    }
}

/// Create directory `path`.  Returns an `FS_*` code.
#[must_use]
pub fn fs_mkdir(path: &str) -> i64 {
    #[cfg(host_fs)]
    {
        if crate::wasm::host_fs_mkdir(path) == 0 {
            FS_OK
        } else {
            FS_OTHER
        }
    }
    #[cfg(not(host_fs))]
    {
        fs_classify(
            crate::file_access::create_dir(crate::file_access::at(path)),
            path,
            false,
        )
    }
}

/// Create directory `path` and all parents.  Returns an `FS_*` code.
#[must_use]
pub fn fs_mkdir_all(path: &str) -> i64 {
    #[cfg(host_fs)]
    {
        if crate::wasm::host_fs_mkdir_all(path) == 0 {
            FS_OK
        } else {
            FS_OTHER
        }
    }
    #[cfg(not(host_fs))]
    {
        fs_classify(
            crate::file_access::create_dir_all(crate::file_access::at(path)),
            path,
            false,
        )
    }
}

/// Returns `true` if `path` exists and is a directory.
#[must_use]
pub fn fs_is_dir(path: &str) -> bool {
    #[cfg(host_fs)]
    {
        crate::wasm::host_fs_is_dir(path)
    }
    #[cfg(not(host_fs))]
    {
        crate::file_access::is_dir(crate::file_access::at(path))
    }
}

/// Returns `true` if `path` is itself a symbolic link (the link, not its target:
/// `fs_is_dir` / `fs_is_file` follow it).  A host filesystem has no links, so it
/// answers `false` there rather than asking the host.
#[must_use]
pub fn fs_is_symlink(path: &str) -> bool {
    #[cfg(host_fs)]
    {
        let _ = path;
        false
    }
    #[cfg(not(host_fs))]
    {
        crate::file_access::is_symlink(crate::file_access::at(path))
    }
}

/// Returns `true` if `path` exists and is a regular file.
#[must_use]
pub fn fs_is_file(path: &str) -> bool {
    #[cfg(host_fs)]
    {
        crate::wasm::host_fs_is_file(path)
    }
    #[cfg(not(host_fs))]
    {
        crate::file_access::is_file(crate::file_access::at(path))
    }
}

// The path-taking operations as a PROGRAM reaches them, in both backends: the program's
// path goes through `Stores::resolve_path` (formal/paths.md), and a refused path answers the
// operation's own failure — `FS_OTHER` for a change, `false` for a question
// (`@FR-Path-Refuse`).  The stdlib's `#rust` templates and the interpreter's handlers call
// these — written `s.database.fs_*_at(…)`, which the native rewriter turns into
// `stores.fs_*_at(…)` — never `resolve_path` and an `fs_*` in two steps.

impl Stores {
    #[must_use]
    pub fn fs_delete_at(&self, raw: &str) -> i64 {
        self.resolve_path(raw).map_or(FS_OTHER, |p| fs_delete(&p))
    }

    #[must_use]
    pub fn fs_move_at(&self, from: &str, to: &str) -> i64 {
        match (self.resolve_path(from), self.resolve_path(to)) {
            (Some(f), Some(t)) => fs_move(&f, &t),
            _ => FS_OTHER,
        }
    }

    #[must_use]
    pub fn fs_mkdir_at(&self, raw: &str) -> i64 {
        self.resolve_path(raw).map_or(FS_OTHER, |p| fs_mkdir(&p))
    }

    #[must_use]
    pub fn fs_mkdir_all_at(&self, raw: &str) -> i64 {
        self.resolve_path(raw)
            .map_or(FS_OTHER, |p| fs_mkdir_all(&p))
    }

    #[must_use]
    pub fn fs_rmdir_at(&self, raw: &str) -> i64 {
        self.resolve_path(raw).map_or(FS_OTHER, |p| fs_rmdir(&p))
    }

    #[must_use]
    pub fn fs_is_dir_at(&self, raw: &str) -> bool {
        self.resolve_path(raw).is_some_and(|p| fs_is_dir(&p))
    }

    #[must_use]
    pub fn fs_is_file_at(&self, raw: &str) -> bool {
        self.resolve_path(raw).is_some_and(|p| fs_is_file(&p))
    }

    /// `mtime(path)`: 0 for a refused path, as for a missing one.
    #[must_use]
    pub fn fs_mtime_at(&self, raw: &str) -> i64 {
        self.resolve_path(raw)
            .map_or(0, |p| Stores::os_mtime_native(&p))
    }

    #[must_use]
    pub fn fs_is_symlink_at(&self, raw: &str) -> bool {
        self.resolve_path(raw).is_some_and(|p| fs_is_symlink(&p))
    }
}
