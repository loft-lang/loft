// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I70 — Database subsystem: the release a program's stores are written by.
//
//! @PLN196 step 9 — a store records the release of the program that wrote it, so a later
//! release knows which of its conversion steps the store still needs.  The release is the
//! entry program's own `loft.toml`: its `version`, the oldest release whose stores it still
//! reads (`data_compatible_with`), and the last release that read older ones
//! (`data_bridge`), which a refusal names as the way forward.

/// The release of the running program, from its project's `loft.toml`.  Every field is empty
/// when the program has no manifest or the manifest does not say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// `[package] version`: the release every store this program writes records.
    pub version: String,
    /// `[package] data_compatible_with`: a store written by an older release is refused.
    pub oldest: String,
    /// `[package] data_bridge`: the last release that still read stores older than
    /// `oldest`, named by the refusal.
    pub bridge: String,
}

impl Release {
    /// The release of the program whose source is in `source_dir`: the nearest `loft.toml`
    /// above it.  Empty when there is none.
    #[must_use]
    pub fn of_source_dir(source_dir: &str) -> Release {
        if source_dir.is_empty() {
            return Release::default();
        }
        let Some(root) = crate::resolution_scope::project_root(source_dir) else {
            return Release::default();
        };
        let toml = root.join("loft.toml");
        let Some(m) = crate::manifest::read_manifest(&toml.to_string_lossy()) else {
            return Release::default();
        };
        Release {
            version: m.version.unwrap_or_default(),
            oldest: m.data_compatible_with.unwrap_or_default(),
            bridge: m.data_bridge.unwrap_or_default(),
        }
    }

    /// Does this release record nothing?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.version.is_empty() && self.oldest.is_empty() && self.bridge.is_empty()
    }

    /// `@FR-L-Release` — may a store written by release `written` be read at all?  `Err` with
    /// the sentence a refusal prints when it is older than the oldest release this program
    /// reads.  A store that records no release predates the recording and is read, as a
    /// sidecar without a `schema=` line is: an existing store is never newly refused.
    ///
    /// # Errors
    /// The refusal, naming the release that bridges the gap when the manifest records one.
    pub fn admits(&self, written: &str) -> Result<(), String> {
        if written.is_empty() || self.oldest.is_empty() {
            return Ok(());
        }
        let (Some(w), Some(o)) = (version_triple(written), version_triple(&self.oldest)) else {
            return Ok(());
        };
        if w >= o {
            return Ok(());
        }
        Err(if self.bridge.is_empty() {
            format!(
                "this store was written by {written}, and this program reads stores from \
                 {} on; open it once with a release that reads both, then with this one",
                self.oldest
            )
        } else {
            format!(
                "this store was written by {written}, and this program reads stores from \
                 {} on; open it once with {}, then with this one",
                self.oldest, self.bridge
            )
        })
    }

    /// Is a store written by `written` older than this release?  A store that records no
    /// release predates the recording, so it is older than any; so is every store of a
    /// program that records none.
    #[must_use]
    pub fn is_newer_than(&self, written: &str) -> bool {
        match (version_triple(&self.version), version_triple(written)) {
            (Some(me), Some(w)) => w < me,
            _ => true,
        }
    }
}

impl crate::database::Stores {
    /// This program's release: what a native build set from the manifest it was compiled
    /// against, else the `loft.toml` above the program's source, read once.
    #[must_use]
    pub fn release(&self) -> &Release {
        self.release
            .get_or_init(|| Release::of_source_dir(&self.source_dir))
    }
}

/// `X`, `X.Y` or `X.Y.Z` as a numeric triple, missing parts 0; `None` when it is not one.
#[must_use]
pub fn version_triple(s: &str) -> Option<(u32, u32, u32)> {
    crate::manifest::parse_version(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(version: &str, oldest: &str, bridge: &str) -> Release {
        Release {
            version: version.into(),
            oldest: oldest.into(),
            bridge: bridge.into(),
        }
    }

    #[test]
    fn a_store_below_the_oldest_release_is_refused_naming_the_bridge() {
        let rel = r("3.0", "2.0", "2.3");
        assert_eq!(
            rel.admits("1.4").unwrap_err(),
            "this store was written by 1.4, and this program reads stores from 2.0 on; \
             open it once with 2.3, then with this one"
        );
        assert!(rel.admits("2.0").is_ok());
        assert!(rel.admits("10.0").is_ok(), "numeric, not text order");
        assert!(rel.admits("").is_ok(), "an unrecorded release is read");
        assert!(r("3.0", "", "").admits("0.1").is_ok(), "no floor declared");
    }

    #[test]
    fn newer_compares_numerically() {
        assert!(r("10.0", "", "").is_newer_than("9.0"));
        assert!(!r("9.0", "", "").is_newer_than("10.0"));
        assert!(!r("2.0", "", "").is_newer_than("2.0"));
        assert!(r("2.0", "", "").is_newer_than(""));
        assert!(r("", "", "").is_newer_than("2.0"));
    }
}
