// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! `loft verify-self` — is this installation the one that was released?
//!
//! @PLN78 step 2.  Read-only: it hashes files and compares, and changes nothing.
//!
//! ## One manifest, one anchor
//!
//! A bundle carries exactly one manifest, `SHA256SUMS`: a sha256 of every file in it,
//! `bin/loft` and every `default/*.loft` included.  It is already the authoritative
//! list elsewhere — [`self_update::owned_files`](crate::self_update) reads it to know
//! which files a bundle owns — so validation reads the same list, and there is one
//! answer to "what did this release ship?" rather than two that can disagree.
//!
//! (An earlier design shipped a second `stdlib.manifest` covering `default/*.loft`.
//! It described a strict subset of what `SHA256SUMS` already covered, which made two
//! ways to validate one installation; the whole reason to have a manifest is that
//! there is one.)
//!
//! Validation is three questions, and it takes all three to mean anything:
//!
//! 1. **Does each listed file still hash to what the manifest says?** — catches a
//!    truncated unpack, an edited stdlib file, a **partial upgrade** (a new `bin/loft`
//!    beside an old `default/`, which runs and misbehaves subtly enough to look like a
//!    compiler bug).
//! 2. **Does `default/` contain a `*.loft` the manifest does not list?** — a digest
//!    check is silent about an ADDED file, and an added stdlib file is not inert:
//!    `cache::collect_stdlib_sources` takes every `*.loft` it finds there, so dropping
//!    one in is enough to have it loaded.  The property is *the stdlib that loads is
//!    the stdlib that shipped*, which needs the file SET, not just the contents.
//! 3. **Does the manifest itself match the signed registry index?** — questions 1 and 2
//!    are answered by a file that ships INSIDE the bundle it describes, so on their own
//!    they establish *intact*, never *authentic*: whoever replaced the binary could
//!    rewrite the manifest beside it.  The registry entry names the manifest's digest
//!    (`binaries.<triple>.manifest_sha256`), and `index.json` is the one thing we sign
//!    — so comparing against it is what closes the chain from a signature down to every
//!    file on disk.
//!
//! An entry that names no digest leaves question 3 **unanswered, and says so**; it is
//! never reported as a pass.

use std::path::{Path, PathBuf};

/// What one check found.  `Skipped` is not a failure: a dev tree legitimately has no
/// release manifest, and saying "not a release bundle" is the useful answer there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Ok(String),
    Failed(String),
    Skipped(String),
}

impl Check {
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self, Check::Failed(_))
    }
}

/// One `<sha256>  <path>` line of a manifest, as `(path, digest)`.
///
/// Tolerates both `sha256sum` and macOS `shasum -a 256` output (identical format), a
/// `combined  <digest>` trailer, and blank lines.  Returns the entries and the
/// `combined` digest when present.
#[must_use]
pub fn parse_manifest(text: &str) -> (Vec<(String, String)>, Option<String>) {
    let mut entries = Vec::new();
    let mut combined = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `sha256sum` writes `<digest>  <path>`; the path may itself contain spaces,
        // so split once and keep the remainder whole.
        let Some((first, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start().trim_start_matches('*'); // `*` = binary-mode marker
        if first == "combined" {
            combined = Some(rest.to_string());
        } else {
            entries.push((rest.to_string(), first.to_string()));
        }
    }
    (entries, combined)
}

/// Does a manifest path reach outside the bundle it describes?
///
/// The ONE home of this rule — `check_manifest` (verification) and the updater's
/// `owned_files` (what an update may replace) both read it, because the two carried
/// separate copies and the copies disagreed by exactly the absolute-path shape:
/// `check_manifest` guarded only `..`, and `root.join("/etc/x")` REPLACES the root, so
/// a hostile manifest made verification read an attacker-named absolute path and
/// disclose its existence in the report.  `..` anywhere is rejected even inside a
/// legal name (`a..b`): a false refusal of a peculiar filename is recoverable, a path
/// that escapes is not.
#[must_use]
pub fn manifest_path_escapes(rel: &str) -> bool {
    rel.contains("..") || Path::new(rel).is_absolute()
}

/// Verify every `<digest>  <path>` entry of `manifest_text` against files under `root`.
///
/// `label` names the manifest in the messages.  A missing file is a failure, not a
/// skip: the manifest says it should be there.
#[must_use]
pub fn check_manifest(root: &Path, manifest_text: &str, label: &str) -> Check {
    let (entries, _) = parse_manifest(manifest_text);
    if entries.is_empty() {
        return Check::Skipped(format!("{label}: no entries"));
    }
    let mut bad: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for (rel, want) in &entries {
        // Never let a manifest path escape the bundle it describes — refused before
        // any filesystem access, so a hostile entry is never even probed.
        if manifest_path_escapes(rel) {
            bad.push(format!("{rel} (rejected: path escapes the bundle)"));
            continue;
        }
        let path = root.join(rel);
        match std::fs::read(&path) {
            Ok(bytes) => {
                if crate::integrity::verify_sha256(&bytes, want).is_err() {
                    bad.push(rel.clone());
                }
            }
            Err(_) => missing.push(rel.clone()),
        }
    }
    if bad.is_empty() && missing.is_empty() {
        return Check::Ok(format!("{label}: {} file(s) match", entries.len()));
    }
    let mut parts = Vec::new();
    if !bad.is_empty() {
        parts.push(format!("{} changed: {}", bad.len(), summarise(&bad)));
    }
    if !missing.is_empty() {
        parts.push(format!(
            "{} missing: {}",
            missing.len(),
            summarise(&missing)
        ));
    }
    Check::Failed(format!("{label}: {}", parts.join("; ")))
}

/// First few names, then a count — a wholly-corrupt bundle must not print 50 lines.
fn summarise(names: &[String]) -> String {
    const SHOW: usize = 3;
    if names.len() <= SHOW {
        return names.join(", ");
    }
    format!(
        "{}, and {} more",
        names[..SHOW].join(", "),
        names.len() - SHOW
    )
}

/// The bundle root for a running binary at `exe`: `<binary-dir>/..`, which is where
/// `default/` sits — the same resolution loft uses to find its stdlib, so this checks
/// the stdlib that would actually be LOADED, not one that merely sits nearby.
#[must_use]
pub fn bundle_root(exe: &Path) -> Option<PathBuf> {
    Some(exe.parent()?.parent()?.to_path_buf())
}

/// Run the local (offline) checks for the installation rooted at `root`.
///
/// Absent manifest → a dev tree or a bare binary, and the answer is "not a release
/// bundle", not a failure.  Does NOT include the registry anchor: that needs a network
/// or a cache, and these are the checks that work with neither.
#[must_use]
pub fn local_checks(root: &Path) -> Vec<Check> {
    let Ok(text) = std::fs::read_to_string(root.join(MANIFEST)) else {
        return vec![Check::Skipped(format!(
            "no {MANIFEST} beside the binary — not a release bundle"
        ))];
    };
    vec![
        check_manifest(root, &text, "files"),
        check_no_extra_stdlib(root, &text),
    ]
}

/// The one manifest a release bundle carries.
pub const MANIFEST: &str = "SHA256SUMS";

/// Reject a `default/*.loft` that no manifest entry accounts for.
///
/// [`check_manifest`] asks "does every listed file still match?", which cannot see a
/// file that was ADDED — and for the stdlib specifically, an added file is not inert.
/// `cache::collect_stdlib_sources` takes every `*.loft` directly under `default/`, so
/// dropping one in is enough to have it parsed and its definitions loaded.  A digest
/// check over the shipped files is silent about it: each of them is untouched.
///
/// Deliberately scoped to `default/`: that is loft's own directory and the one whose
/// extra files execute.  The bundle root cannot be checked the same way, because a
/// `--prefix ~/.local` install shares it with everything else the user keeps there, and
/// every one of those would read as an intruder.
fn check_no_extra_stdlib(root: &Path, manifest_text: &str) -> Check {
    let (entries, _) = parse_manifest(manifest_text);
    let listed: std::collections::HashSet<&str> = entries
        .iter()
        .filter_map(|(rel, _)| rel.strip_prefix("default/"))
        .collect();
    if listed.is_empty() {
        return Check::Skipped("stdlib set: the manifest lists no default/ files".to_string());
    }
    let Ok(read) = std::fs::read_dir(root.join("default")) else {
        // `check_manifest` already reports the listed files as missing; one unreadable
        // directory should not be announced twice.
        return Check::Skipped("stdlib set: no default/ directory".to_string());
    };
    let mut extra: Vec<String> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("loft") {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && !listed.contains(name)
        {
            extra.push(name.to_string());
        }
    }
    if extra.is_empty() {
        return Check::Ok(format!("stdlib set: {} file(s), none added", listed.len()));
    }
    extra.sort();
    Check::Failed(format!(
        "stdlib set: {} file(s) in default/ that the release did not ship, and loft \
         loads every *.loft it finds there: {}",
        extra.len(),
        summarise(&extra)
    ))
}

/// The digest of this installation's manifest — the value the registry entry names.
///
/// One hash over one file is the whole comparison: `SHA256SUMS` covers every file in
/// the bundle, so establishing that IT is what the release published establishes the
/// same for everything it lists (which [`check_manifest`] then verifies on disk).
#[must_use]
pub fn manifest_digest(root: &Path) -> Option<String> {
    std::fs::read(root.join(MANIFEST))
        .ok()
        .map(|b| crate::integrity::sha256_hex(&b))
}

/// Compare this installation's manifest against the digest the signed index publishes.
///
/// `published` is `None` when the registry names no digest for this build — an older
/// entry, a version never published, or a bundle installed by hand.  That is reported
/// as unanswered rather than as a pass: "we could not check where this came from" and
/// "this came from us" are the two things a user must never have to tell apart.
#[must_use]
pub fn check_anchor(root: &Path, published: Option<&str>) -> Check {
    let Some(local) = manifest_digest(root) else {
        return Check::Skipped(format!("origin: no {MANIFEST} to compare"));
    };
    let Some(want) = published else {
        return Check::Skipped(
            "origin: the registry names no manifest digest for this build — \
             intact, but not traced to a signature"
                .to_string(),
        );
    };
    if local.eq_ignore_ascii_case(want) {
        Check::Ok("origin: matches the signed registry index".to_string())
    } else {
        Check::Failed(format!(
            "origin: this installation does not match the signed registry index \
             (index {want}, here {local}) — the files may be intact and still not be \
             the release they claim to be"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join("loft-verify-self-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Both manifest dialects and the `combined` trailer parse.
    #[test]
    fn manifest_parses_both_dialects_and_the_combined_trailer() {
        let (e, c) = parse_manifest(
            "abc123  default/01_code.loft\ndef456 *default/02_files.loft\n\ncombined  ff00\n",
        );
        assert_eq!(
            e,
            vec![
                ("default/01_code.loft".to_string(), "abc123".to_string()),
                ("default/02_files.loft".to_string(), "def456".to_string()),
            ]
        );
        assert_eq!(c.as_deref(), Some("ff00"));
    }

    /// An intact bundle passes.
    #[test]
    fn an_intact_bundle_verifies() {
        let d = scratch("intact");
        write(&d, "default/a.loft", "fn main() {}\n");
        let digest = crate::integrity::sha256_hex(b"fn main() {}\n");
        let m = format!("{digest}  default/a.loft\n");
        assert!(matches!(check_manifest(&d, &m, "stdlib"), Check::Ok(_)));
    }

    /// The positive control, and the failure this exists to catch: an edited stdlib
    /// file. A check that cannot fail is not a check.
    #[test]
    fn an_edited_file_fails_and_is_named() {
        let d = scratch("edited");
        write(&d, "default/a.loft", "fn main() {}\n");
        let m = format!(
            "{}  default/a.loft\n",
            crate::integrity::sha256_hex(b"something else\n")
        );
        let Check::Failed(msg) = check_manifest(&d, &m, "stdlib") else {
            panic!("an edited file must FAIL");
        };
        assert!(msg.contains("default/a.loft"), "must name the file: {msg}");
        assert!(msg.contains("changed"), "must say what is wrong: {msg}");
    }

    /// A partial upgrade — the manifest lists a file the installation lost. Reported
    /// as missing rather than silently passing over it.
    #[test]
    fn a_missing_file_fails_as_missing() {
        let d = scratch("missing");
        let m = "00  default/gone.loft\n";
        let Check::Failed(msg) = check_manifest(&d, m, "stdlib") else {
            panic!("a missing file must FAIL");
        };
        assert!(msg.contains("missing"), "{msg}");
        assert!(msg.contains("default/gone.loft"), "{msg}");
    }

    /// A manifest path may not reach outside the bundle it describes.
    #[test]
    fn a_manifest_path_cannot_escape_the_bundle() {
        let d = scratch("escape");
        let m = "00  ../../etc/passwd\n";
        let Check::Failed(msg) = check_manifest(&d, m, "bundle") else {
            panic!("an escaping path must FAIL");
        };
        assert!(msg.contains("escapes the bundle"), "{msg}");
    }

    /// A dev tree has no manifest — "not a release bundle", not a failure.
    #[test]
    fn a_tree_without_a_manifest_is_skipped_not_failed() {
        let d = scratch("dev");
        let checks = local_checks(&d);
        assert!(
            checks.iter().all(|c| matches!(c, Check::Skipped(_))),
            "{checks:?}"
        );
        assert!(!checks.iter().any(Check::failed));
    }

    /// Write a minimal bundle: one stdlib file, and the manifest that describes it.
    fn bundle(name: &str) -> PathBuf {
        let d = scratch(name);
        write(&d, "default/01_code.loft", "fn a() {}\n");
        std::fs::write(
            d.join(MANIFEST),
            format!(
                "{}  default/01_code.loft\n",
                crate::integrity::sha256_hex(b"fn a() {}\n")
            ),
        )
        .unwrap();
        d
    }

    /// The hole a digest check cannot see: a file ADDED to `default/`.  Every shipped
    /// file still matches, so question 1 passes; loft would load the intruder anyway.
    #[test]
    fn an_added_stdlib_file_is_caught_even_though_every_shipped_file_matches() {
        let d = bundle("added");
        // Control: intact first, so a failure below cannot be blamed on the fixture.
        assert!(
            !local_checks(&d).iter().any(Check::failed),
            "fixture must start clean"
        );

        write(&d, "default/99_evil.loft", "fn evil() {}\n");
        let checks = local_checks(&d);
        assert!(
            matches!(&checks[0], Check::Ok(_)),
            "every SHIPPED file still matches: {:?}",
            checks[0]
        );
        let Check::Failed(msg) = &checks[1] else {
            panic!("an added stdlib file must FAIL: {checks:?}");
        };
        assert!(msg.contains("99_evil.loft"), "{msg}");
    }

    /// A non-`.loft` file next to the stdlib is not loaded, so it is not an intruder.
    #[test]
    fn an_unrelated_file_in_default_is_not_flagged() {
        let d = bundle("unrelated");
        write(&d, "default/notes.txt", "scratch\n");
        assert!(!local_checks(&d).iter().any(Check::failed));
    }

    /// The anchor: the same intact bundle reads differently depending on what the
    /// signed index says about it — and "no digest published" is never a pass.
    #[test]
    fn the_anchor_distinguishes_intact_from_authentic() {
        let d = bundle("anchor");
        let digest = manifest_digest(&d).expect("a bundle has a manifest digest");

        assert!(matches!(check_anchor(&d, Some(&digest)), Check::Ok(_)));

        let Check::Failed(msg) = check_anchor(&d, Some(&"0".repeat(64))) else {
            panic!("a digest the index does not name must FAIL");
        };
        assert!(msg.contains("signed registry index"), "{msg}");

        let Check::Skipped(msg) = check_anchor(&d, None) else {
            panic!("an unpublished digest is UNANSWERED, never a pass");
        };
        assert!(msg.contains("not traced to a signature"), "{msg}");

        // Non-vacuity: the digest is of the manifest, so editing it changes the answer.
        std::fs::write(d.join(MANIFEST), "0000  default/01_code.loft\n").unwrap();
        assert!(check_anchor(&d, Some(&digest)).failed());
    }

    /// The stdlib is checked where loft LOADS it from: `<binary-dir>/../default`.
    #[test]
    fn bundle_root_is_the_parent_of_bin() {
        let root = bundle_root(Path::new("/opt/loft/bin/loft")).unwrap();
        assert_eq!(root, Path::new("/opt/loft"));
        assert!(root.join("default").ends_with("loft/default"));
    }
}
