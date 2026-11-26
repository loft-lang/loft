// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! The registry cache's signed documents (the index and the advisory feed): how old the copy
//! in use is, the copy a loft bundle ships for a machine that has none, and placing a copy so
//! that the cache never goes back to an older one.
//!
//! A signature says who wrote a document, not when it was read.  So beside each cached
//! document the cache keeps the moment that copy was last known to be current —
//! `<doc>.verified_at`, in Unix seconds — written when a fetch verified it; a seed or a bundle
//! brings its own.  The age of the copy in use is measured from that stamp.  It is not the
//! file's modification time, which every copy resets: a seed placed today from a bundle built
//! months ago would read as fresh, and the registry would not be asked for a day.  Nor is it
//! the document's own `updated` stamp, which says when the registry last CHANGED it: a feed
//! with no new advisory for a month is current all that month.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};

use crate::file_access;
use crate::registry_index;

/// The age past which the copy in use is reported.
pub const WARN_AGE_SECS: u64 = 7 * 86_400;
/// The age past which `loft install` will not go on reading it, unless told to.
pub const REFUSE_AGE_SECS: u64 = 30 * 86_400;

/// The seed's directory, beside `bin/` in a loft bundle.
const SEED_DIR: &str = "registry";
/// The seed's record of the registry it was read from, one URL.
const SEED_SOURCE: &str = "source";
/// The seed's record of when it was read, Unix seconds.
const SEED_VERIFIED_AT: &str = "verified_at";

/// Now, in Unix seconds.
#[must_use]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Where the moment `doc` was last known current is kept: `<doc>.verified_at`.
#[must_use]
pub fn verified_at_path(doc: &Path) -> PathBuf {
    let mut p = doc.as_os_str().to_owned();
    p.push(".verified_at");
    PathBuf::from(p)
}

/// Record that the copy at `doc` was known current at `secs`.  A stamp that cannot be written
/// leaves the age measured from the file's modification time, as before the stamp existed.
pub fn stamp(doc: &Path, secs: u64) {
    let _ = registry_index::replace_atomically(&verified_at_path(doc), secs.to_string().as_bytes());
}

/// When the copy at `doc` was last known current: its stamp, else the file's modification time
/// (a cache written before the stamp existed).  `None` when there is no copy.
#[must_use]
pub fn verified_at(doc: &Path) -> Option<u64> {
    let meta = file_access::metadata(doc).ok()?;
    if let Some(secs) = file_access::read_to_string(verified_at_path(doc))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
    {
        return Some(secs);
    }
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// The age of the copy at `doc` in seconds; `None` when there is no copy.
#[must_use]
pub fn age_secs(doc: &Path) -> Option<u64> {
    verified_at(doc).map(|at| now_secs().saturating_sub(at))
}

/// Whether the copy at `doc` is more than `ttl` seconds old, or absent.
#[must_use]
pub fn older_than(doc: &Path, ttl: u64) -> bool {
    age_secs(doc).is_none_or(|age| age > ttl)
}

/// `YYYY-MM-DDThh:mm:ssZ` (the registry's `updated` and `published` form) in Unix seconds;
/// `None` for anything else.
#[must_use]
pub fn rfc3339_secs(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Days from 1970-01-01 to y-m-d (Howard Hinnant's `days_from_civil`).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// The latest a copy of `content` with no stamp of its own can have been current: the
/// document's signed `updated` (it cannot have been read before it was written).  `None`
/// without one.
#[must_use]
pub fn updated_secs(content: &[u8]) -> Option<u64> {
    rfc3339_secs(&registry_index::updated_stamp(content))
}

/// What the age of the copy in use means for an install ([`age_verdict`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Age {
    /// Recent enough to say nothing.
    Fresh,
    /// Past [`WARN_AGE_SECS`]: the line to print.
    Old(String),
    /// Past [`REFUSE_AGE_SECS`] and not waived: why the install stops.
    TooOld(String),
}

/// The verdict on a `what` (the registry index, the advisory feed) whose copy in use is
/// `age_secs` old.  `waived` is the caller's `--offline` or `--allow-stale`: a machine with no
/// network, or a person who has said the old copy will do, keeps going with a warning.
#[must_use]
pub fn age_verdict(what: &str, age_secs: u64, waived: bool) -> Age {
    let days = age_secs / 86_400;
    if age_secs > REFUSE_AGE_SECS && !waived {
        Age::TooOld(format!(
            "the {what} in use was last read from the registry {days} days ago, and the \
             registry could not be reached to read it again; run the command again with a \
             network connection, or pass --allow-stale to install with the copy you have"
        ))
    } else if age_secs > WARN_AGE_SECS {
        Age::Old(format!(
            "warning: the {what} in use was last read from the registry {days} days ago"
        ))
    } else {
        Age::Fresh
    }
}

/// The seed this loft carries for the registry in use: the directory named by
/// `LOFT_REGISTRY_SEED`, else `registry/` beside the bundle's `bin/`.  It holds the registry's
/// signed `index.json` and `advisories.json` with their signatures as the registry serves
/// them, `source` (the registry URL they were read from) and `verified_at` (when).  `None`
/// when there is none, or it was read from another registry than the one in use.
#[must_use]
pub fn seed() -> Option<PathBuf> {
    let dir = match std::env::var_os("LOFT_REGISTRY_SEED") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => std::env::current_exe()
            .ok()
            .and_then(|e| file_access::try_plain_canonical(e.as_path()))?
            .parent()?
            .parent()?
            .join(SEED_DIR),
    };
    let source = file_access::read_to_string(dir.join(SEED_SOURCE)).ok()?;
    (source.trim() == registry_index::registry_url()).then_some(dir)
}

/// The seed a bundle rooted at `root` ships (`root/registry/`), checked against this loft's
/// registry keys: one `(verified, line)` per document, none when the bundle ships no seed.
/// `loft verify-self` reports them, and the release build runs it, so a seed whose signature
/// does not verify is never shipped.
#[must_use]
pub fn seed_checks(root: &Path) -> Vec<(bool, String)> {
    use crate::registry_signing::{Purpose, VerifyResult, verify_document};
    let dir = root.join(SEED_DIR);
    if !file_access::exists(dir.join(SEED_SOURCE)) {
        return Vec::new();
    }
    [
        ("index.json", Purpose::Index),
        ("advisories.json", Purpose::Advisories),
    ]
    .into_iter()
    .map(|(name, purpose)| {
        let verdict = file_access::read(dir.join(name)).map(|content| {
            let sig = registry_index::read_signatures(&dir.join(format!("{name}.sig")));
            verify_document(purpose, &content, &sig)
        });
        match verdict {
            Ok(VerifyResult::Valid) => (
                true,
                format!("registry seed: {name} is signed by the registry"),
            ),
            Ok(other) => {
                let why = match other {
                    VerifyResult::NoTrustRoot => "this loft carries no registry keys",
                    VerifyResult::MalformedSignature => "its signature is missing or malformed",
                    _ => "its signature does not match",
                };
                (
                    false,
                    format!("registry seed: {name} does not verify — {why}"),
                )
            }
            Err(e) => (false, format!("registry seed: {name} cannot be read ({e})")),
        }
    })
    .collect()
}

/// Whether the registry in use is known to serve an advisory feed, so a feed it answers it
/// does not have is a failure and never "not published yet": the default registry does, and
/// so does one this loft carries a seed of.
#[must_use]
pub fn registry_serves_a_feed() -> bool {
    registry_index::registry_url() == registry_index::DEFAULT_REGISTRY_URL || seed().is_some()
}

/// Bring the seed's `name` (`index.json`, `advisories.json`) into the cache at `doc` /
/// `sig_path` when the cache holds no copy, or an older one.  `verify` is the document's own
/// check; a seed copy that fails it is not placed.  Silent: a seed that cannot be used leaves
/// the cache as it was, and the fetch that follows decides.
pub fn seed_into(
    name: &str,
    doc: &Path,
    sig_path: &Path,
    verify: &mut dyn FnMut(&[u8], &[u8]) -> bool,
) {
    let Some(dir) = seed() else {
        return;
    };
    let Ok(content) = file_access::read(dir.join(name)) else {
        return;
    };
    if let Ok(cached) = file_access::read(doc) {
        let (seeded, have) = (
            registry_index::updated_stamp(&content),
            registry_index::updated_stamp(&cached),
        );
        if seeded.is_empty() || seeded <= have {
            return;
        }
    }
    let signature = registry_index::read_signatures(&dir.join(format!("{name}.sig")));
    if !verify(&content, &signature) {
        return;
    }
    let at = file_access::read_to_string(dir.join(SEED_VERIFIED_AT))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .or_else(|| updated_secs(&content))
        .unwrap_or(0);
    let _ = place_pair(name, doc, sig_path, &content, &signature, at, verify);
}

/// Why `content` may not replace the cached copy at `doc`: that copy verifies and is NEWER, by
/// the documents' signed `updated` stamps.  `None` when it may.  `cached_verifies` checks the
/// copy already there; one that does not verify is no copy.
#[must_use]
pub fn newer_copy_cached(
    what: &str,
    doc: &Path,
    sig_path: &Path,
    content: &[u8],
    cached_verifies: &mut dyn FnMut(&[u8], &[u8]) -> bool,
) -> Option<String> {
    let cached = file_access::read(doc).ok()?;
    if !cached_verifies(&cached, &registry_index::read_signatures(sig_path)) {
        return None;
    }
    let (new, old) = (
        registry_index::updated_stamp(content),
        registry_index::updated_stamp(&cached),
    );
    (!new.is_empty() && !old.is_empty() && new < old).then(|| {
        format!(
            "the {what} of {new} is older than the one already verified here ({old}); \
             the cache keeps its copy"
        )
    })
}

/// Place a verified pair in the cache, known current at `verified_at`, unless the cache holds
/// a verified copy that is newer ([`newer_copy_cached`]): a cache only moves forward, whatever
/// brought the copy — a fetch, a seed, a bundle carried by hand.
///
/// # Errors
/// Why the pair was not placed: the cache holds a newer copy, or a file could not be written.
pub fn place_pair(
    what: &str,
    doc: &Path,
    sig_path: &Path,
    content: &[u8],
    signature: &[u8],
    verified_at: u64,
    cached_verifies: &mut dyn FnMut(&[u8], &[u8]) -> bool,
) -> Result<(), String> {
    if let Some(why) = newer_copy_cached(what, doc, sig_path, content, cached_verifies) {
        return Err(why);
    }
    registry_index::write_signed_pair(doc, sig_path, content, signature)
        .map_err(|e| format!("cache {what}: {e}"))?;
    stamp(doc, verified_at);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_rfc3339_stamp_reads_as_unix_seconds() {
        assert_eq!(rfc3339_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_secs("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(rfc3339_secs("2026-11-20T21:30:00Z"), Some(1_795_210_200));
        assert_eq!(rfc3339_secs("2026-11-20"), None);
        assert_eq!(rfc3339_secs("2026-13-20T21:30:00Z"), None);
    }

    /// B's thresholds: silent to 7 days, a warning past them, a refusal past 30 unless waived
    /// (`--offline`, `--allow-stale`), which still warns.
    #[test]
    fn the_age_of_the_copy_in_use_warns_then_refuses() {
        let day = 86_400;
        assert_eq!(age_verdict("advisory feed", 7 * day, false), Age::Fresh);
        assert!(matches!(
            age_verdict("advisory feed", 8 * day, false),
            Age::Old(w) if w.contains("8 days ago")
        ));
        assert!(matches!(
            age_verdict("advisory feed", 30 * day, false),
            Age::Old(_)
        ));
        assert!(matches!(
            age_verdict("advisory feed", 31 * day, false),
            Age::TooOld(w) if w.contains("--allow-stale") && w.contains("31 days ago")
        ));
        assert!(matches!(
            age_verdict("advisory feed", 31 * day, true),
            Age::Old(_)
        ));
    }
}
