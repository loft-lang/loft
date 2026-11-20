// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I52 — Persistence: the guard beside a saved store

//! `@FR-S-Guard` — the guard beside a store image: what the image and its sidecar hashed to
//! when they were written, checked before a loader reads either.
//!
//! It answers one question: are these the bytes that were written?  A save cut short by a
//! power loss, a damaged file, an image beside another write's sidecar — each is refused
//! instead of read.  It is not a lock: whoever changes a save on purpose removes the guard or
//! writes a new one, and the load goes on (`@FR-S-Sound` is what keeps a changed save from
//! reading outside itself).  A store with no guard loads as it always did, and a save writes
//! none: a guard is written by whoever wants one — a tool, a build step, the program — with
//! [`write_beside`] or by hand, and a save over a guarded store removes the guard it outdates.
//!
//! ```text
//! LOFT-GUARD v1
//! image=<SHA-256 of the image, 64 hex digits>
//! sidecar=<SHA-256 of the .dschema sidecar>
//! ```
//!
//! A reader takes the lines it knows and passes over the rest, so a later writer may add
//! lines without an earlier reader refusing its stores.

use std::path::{Path, PathBuf};

const MAGIC: &str = "LOFT-GUARD v";

/// The guard beside the image at `image`: `<image>.guard`.
#[must_use]
pub fn guard_path(image: &Path) -> PathBuf {
    let mut p = image.as_os_str().to_owned();
    p.push(".guard");
    PathBuf::from(p)
}

/// The guard's text for an image and its sidecar.
#[must_use]
pub fn render(image: &[u8], sidecar: &[u8]) -> String {
    format!(
        "{MAGIC}1\nimage={}\nsidecar={}\n",
        crate::integrity::sha256_hex(image),
        crate::integrity::sha256_hex(sidecar)
    )
}

/// Write the guard for the store at `path`: `image` is what was written there, and its
/// sidecar is read back from beside it.  Every writer of an image writes this after the
/// pair, or the guard an earlier write left would refuse the new one.
///
/// # Errors
/// The sidecar cannot be read, or the guard cannot be written.
pub fn write_beside(path: &Path, image: &[u8]) -> std::io::Result<()> {
    let sidecar = crate::file_access::read(crate::schema_sidecar::dschema_path(path))?;
    crate::file_access::write(guard_path(path), render(image, &sidecar))
}

/// Whether `image`, the bytes read from `path`, are what the guard beside `path` was written
/// for; a store with no guard is not asked.
///
/// # Errors
/// Why the store is refused: its guard cannot be read or is not one, or the image or its
/// sidecar is not the one the guard names.
pub fn check(path: &Path, image: &[u8]) -> Result<(), String> {
    let guard = guard_path(path);
    if !crate::file_access::exists(&guard) {
        return Ok(());
    }
    let Ok(text) = crate::file_access::read_to_string(&guard) else {
        return Err("its guard cannot be read".to_string());
    };
    let mut lines = text.lines();
    if !lines.next().is_some_and(|first| first.starts_with(MAGIC)) {
        return Err("its guard is not a guard file".to_string());
    }
    let (mut of_image, mut of_sidecar) = (None, None);
    for line in lines {
        if let Some(hex) = line.strip_prefix("image=") {
            of_image = Some(hex.trim());
        } else if let Some(hex) = line.strip_prefix("sidecar=") {
            of_sidecar = Some(hex.trim());
        }
    }
    let Some(of_image) = of_image else {
        return Err("its guard names no image".to_string());
    };
    if !crate::integrity::sha256_hex(image).eq_ignore_ascii_case(of_image) {
        return Err(format!(
            "it is not the file its guard was written for: damaged, cut short, or changed \
             since"
        ));
    }
    if let Some(of_sidecar) = of_sidecar {
        let sidecar = crate::file_access::read(crate::schema_sidecar::dschema_path(path))
            .map_err(|_| format!("its layout sidecar cannot be read"))?;
        if !crate::integrity::sha256_hex(&sidecar).eq_ignore_ascii_case(of_sidecar) {
            return Err(format!(
                "its layout sidecar is not the one its guard was written for"
            ));
        }
    }
    Ok(())
}
