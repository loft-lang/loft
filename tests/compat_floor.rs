// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Step 1 of the library compatibility contract: the two declared floors parse, and a
//! malformed one is rejected LOUDLY.
//!
//! `api_compatible_with` / `data_compatible_with` name the oldest release of this package
//! that this one is still compatible with — a real version, so the claim is checkable by
//! fetching that release and running its own tests. Nothing reads them yet; this asserts the
//! parse and the validation, so the later gates have something trustworthy to stand on.
//!
//! The loud-rejection cases are the point. `check_version` used to accept `"garbage"` as
//! "any version", which made every library's declared loft bound permanently vacuous — a
//! claim nobody could parse read as a claim nobody needed to check. A compatibility floor
//! fails the same way and worse: silently treating it as "undeclared" would turn a break the
//! author *did* declare into one nobody gates.
//!
//! Design: `doc/claude/plans/library-compat-contract/README.md`.

use loft::file_access as fa;
use loft::manifest::{FloorCheck, check_floor, read_manifest};

/// Round-trip through the real `read_manifest` entry point rather than an internal parser,
/// so the test exercises the path a package actually takes.
fn manifest_of(tag: &str, content: &str) -> loft::manifest::Manifest {
    let dir = std::env::temp_dir().join(format!("loft_floor_{tag}_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("loft.toml");
    fa::write(&path, content).expect("write manifest");
    let m = read_manifest(path.to_str().expect("utf-8 path")).expect("manifest parses");
    let _ = fa::remove_dir_all(&dir);
    m
}

/// Both floors survive a round trip through the real manifest parser, alongside the fields
/// that already exist — they are additive `[package]` keys, not a new section.
#[test]
fn both_floors_parse_from_a_manifest() {
    let m = manifest_of(
        "demo",
        "[package]\n\
         name = \"demo\"\n\
         version = \"0.7.0\"\n\
         loft = \">=0.8\"\n\
         api_compatible_with = \"0.3.0\"\n\
         data_compatible_with = \"0.1.0\"\n\
         \n[library]\nentry = \"src/demo.loft\"\n",
    );
    assert_eq!(m.api_compatible_with.as_deref(), Some("0.3.0"));
    assert_eq!(m.data_compatible_with.as_deref(), Some("0.1.0"));
    assert_eq!(
        m.version.as_deref(),
        Some("0.7.0"),
        "existing fields intact"
    );
    assert_eq!(m.entry.as_deref(), Some("src/demo.loft"));
}

/// A manifest without them parses exactly as before. The fields are optional and additive,
/// so every published package keeps loading — which is what makes this step safe to land
/// with nothing else in place.
#[test]
fn a_manifest_without_floors_is_unchanged() {
    let m = manifest_of(
        "old",
        "[package]\nname = \"old\"\nversion = \"1.0.0\"\nloft = \">=0.8\"\n",
    );
    assert!(m.api_compatible_with.is_none());
    assert!(m.data_compatible_with.is_none());
    assert_eq!(m.name.as_deref(), Some("old"));
}

/// Undeclared is a real state, distinct from malformed: a package that says nothing is
/// grandfathered, and must not be reported as if it made a broken claim.
#[test]
fn undeclared_is_not_malformed() {
    assert_eq!(check_floor(None, Some("1.0.0")), FloorCheck::Undeclared);
}

/// A well-formed floor at or below the package's own version is accepted, in every spelling
/// the version parser takes.
#[test]
fn a_floor_at_or_below_own_version_is_accepted() {
    for (floor, own) in [
        ("0.3.0", "0.7.0"),
        ("0.3", "0.7.0"),
        ("1", "1.0.0"),
        ("2.4.9", "2.4.9"), // equal: "I broke it this release" is legal
    ] {
        assert!(
            matches!(check_floor(Some(floor), Some(own)), FloorCheck::Ok(_)),
            "floor {floor} against own {own} must be accepted"
        );
    }
}

/// The case that motivates validating at all: a floor that is not a version must be REJECTED,
/// never quietly treated as "no floor declared". Silently degrading an unparseable claim to
/// "nothing to check" is how `check_version`'s `>=0.8` became permanently vacuous across the
/// whole registry.
#[test]
fn a_malformed_floor_is_rejected_loudly() {
    for bad in ["garbage", "", "1.2.3.4", "v1.0", "latest", "0.x"] {
        let got = check_floor(Some(bad), Some("9.9.9"));
        assert!(
            matches!(got, FloorCheck::Malformed(_)),
            "`{bad}` must be Malformed, got {got:?}"
        );
    }
}

/// A floor NEWER than the package's own version cannot be honest: the release it names does
/// not exist, so the claim could never be verified by fetching it. Caught here rather than
/// at the fetch, where it would surface as a confusing 404.
#[test]
fn a_floor_above_own_version_is_rejected() {
    let got = check_floor(Some("2.0.0"), Some("1.4.0"));
    assert!(
        matches!(got, FloorCheck::AboveSelf { .. }),
        "a floor above own version must be rejected, got {got:?}"
    );
}

/// An unparseable OWN version is `check_version`'s business, not this check's — so the floor
/// is still validated on its own terms rather than swallowed by a neighbouring error.
#[test]
fn an_unparseable_own_version_does_not_swallow_the_floor_check() {
    assert!(matches!(
        check_floor(Some("0.3.0"), Some("not-a-version")),
        FloorCheck::Ok(_)
    ));
    assert!(matches!(
        check_floor(Some("also-garbage"), Some("not-a-version")),
        FloorCheck::Malformed(_)
    ));
}
