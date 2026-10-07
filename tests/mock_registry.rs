// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLAN12 Phase 6.12 — exercises the mock-registry fixture through
// the parser + classifier code paths.  Verifies the registry-
// resolution machinery (index parse, advisory classify) works
// offline against a file:// URL.

#![cfg(feature = "registry")]

use loft::file_access as fa;
use loft::registry_advisories::{self, Severity};
use loft::registry_index;

fn fixture_path(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mock-registry")
        .join(rel)
}

#[test]
fn mock_index_parses() {
    let content = fa::read_to_string(fixture_path("index.json")).unwrap();
    let idx = registry_index::parse_index(&content).expect("parse mock index");
    assert!(idx.packages.contains_key("test_alpha"));
    assert!(idx.packages.contains_key("test_beta"));
    let alpha = &idx.packages["test_alpha"];
    assert_eq!(alpha.yanked, vec!["0.1.0".to_string()]);
    assert_eq!(alpha.versions.len(), 2);
}

#[test]
fn mock_advisories_classify() {
    let content = fa::read_to_string(fixture_path("advisories.json")).unwrap();
    let feed = registry_advisories::parse_advisories(&content).expect("parse mock advisories");
    assert_eq!(feed.advisories.len(), 2);

    let critical_hits = registry_advisories::classify("test_alpha", "0.1.0", &feed);
    assert_eq!(critical_hits.len(), 1);
    assert_eq!(critical_hits[0].severity, Severity::SecurityCritical);

    let bug_hits = registry_advisories::classify("test_beta", "0.1.0", &feed);
    assert_eq!(bug_hits.len(), 1);
    assert_eq!(bug_hits[0].severity, Severity::Bug);

    // Fixed version: no hits.
    let fixed = registry_advisories::classify("test_alpha", "0.2.0", &feed);
    assert!(fixed.is_empty(), "0.2.0 is the fix; should be silent");
}

#[test]
fn find_best_version_skips_yanked() {
    let content = fa::read_to_string(fixture_path("index.json")).unwrap();
    let idx = registry_index::parse_index(&content).unwrap();
    let alpha = &idx.packages["test_alpha"];
    // 0.1.0 is yanked → best is 0.2.0.
    let best =
        registry_index::find_best_version(alpha, "*", false).expect("at least one non-yanked");
    assert_eq!(best.semver, "0.2.0");
}

#[test]
fn file_url_fetches_local_index() {
    // The http_get_bytes file:// branch should let
    // registry_index::fetch_index work against a file:// URL.
    let url = format!("file://{}", fixture_path("index.json").display());
    let fetched = registry_index::fetch_index(&url).expect("file:// fetch");
    assert!(!fetched.content.is_empty());
    let text = std::str::from_utf8(&fetched.content).unwrap();
    let idx = registry_index::parse_index(text).expect("re-parse");
    assert!(idx.packages.contains_key("test_alpha"));
}

// ── The toolchain entry (@PLN78 1b) ───────────────────────────────
//
// `scripts/gen-toolchain-entry.py` produces the registry entry that makes the
// toolchain installable and self-updatable.  That entry is written once by a script
// and then SIGNED, so a shape mistake in it is discovered at the user, after the
// signature has already attested to it.  These tests close that gap by running the
// generator and putting its output through the real parser and the real planner —
// the two things that will consume it in production.

/// Build a directory of stand-in release artifacts and run the generator over it.
/// The bytes are arbitrary; what is under test is the entry's shape, and a real
/// 18 MB source zip would only make the test slow.
fn generated_toolchain_entry(dir: &std::path::Path, version: &str) -> String {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fa::create_dir_all(dir).unwrap();
    fa::write(dir.join(format!("loft-{version}-src.zip")), b"source").unwrap();
    // Real zips, each carrying a `SHA256SUMS`: the generator reads the manifest out of
    // the bundle to derive `manifest_sha256`, so a stand-in byte string would exercise a
    // different code path than the one that runs at release time.
    for triple in loft::self_update::PUBLISHED_TRIPLES {
        let zip = dir.join(format!("loft-{version}-{triple}.zip"));
        let script = format!(
            "import zipfile,sys\n\
             z=zipfile.ZipFile(sys.argv[1],'w')\n\
             z.writestr('loft-{version}-{triple}/SHA256SUMS','{triple}  default/a.loft\\n')\n\
             z.close()\n"
        );
        let out = loft::platform::process::harness_command("python3")
            .args(["-c", &script])
            .arg(&zip)
            .output()
            .expect("build a fixture zip");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let out = loft::platform::process::harness_command("python3")
        .arg(root.join("scripts/gen-toolchain-entry.py"))
        .args(["--version", version])
        .arg("--dir")
        .arg(dir)
        .args(["--published", "2026-07-31T00:00:00Z"])
        .current_dir(&root)
        .output()
        .expect("run gen-toolchain-entry.py");
    assert!(
        out.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn generated_toolchain_entry_parses_and_drives_self_update() {
    let dir = std::env::temp_dir().join("loft-toolchain-entry-parse");
    let _ = fa::remove_dir_all(&dir);
    let entry = generated_toolchain_entry(&dir, "2026.7.2");

    // Splice it into an index exactly as the registry PR will.
    let index = format!(
        r#"{{"schema_version": 1, "updated": "2026-07-31T00:00:00Z", "packages": {}}}"#,
        entry
    );
    let idx = registry_index::parse_index(&index).expect("the entry we publish must parse");
    let pkg = idx
        .packages
        .get(loft::self_update::TOOLCHAIN_PKG)
        .expect("the toolchain package must be keyed `loft`");
    let ver = &pkg.versions["2026.7.2"];
    assert!(
        ver.url.ends_with("-src.zip"),
        "version artifact is the source: {}",
        ver.url
    );
    assert_eq!(ver.size, 6, "size must be the source zip's real size");
    assert_eq!(
        ver.binaries.len(),
        loft::self_update::PUBLISHED_TRIPLES.len(),
        "every published triple must reach the entry"
    );
    for bin in ver.binaries.values() {
        // A cdylib ABI fingerprint on a toolchain bundle would be compared by
        // `loft install` and mean nothing; its absence is load-bearing.
        assert!(
            bin.loft_ffi_fp.is_none(),
            "toolchain binaries carry no loft_ffi_fp"
        );
        // The anchor `verify-self` needs: without it an installed tree can only be
        // checked against a manifest that shipped inside it.
        let anchor = bin
            .manifest_sha256
            .as_deref()
            .expect("every binary must name its manifest digest");
        assert_eq!(
            anchor.len(),
            64,
            "manifest_sha256 must be a sha256: {anchor}"
        );
    }
    // Each target ships a different binary, so each bundle's manifest differs -- one
    // shared digest would mean the generator hashed something other than the bundle.
    let anchors: std::collections::HashSet<_> = ver
        .binaries
        .values()
        .filter_map(|b| b.manifest_sha256.clone())
        .collect();
    assert_eq!(
        anchors.len(),
        ver.binaries.len(),
        "each bundle must be anchored to its OWN manifest"
    );

    // The planner must act on it: a newer release, built for this host.
    let host = loft::self_update::PUBLISHED_TRIPLES[0];
    match loft::self_update::plan(&idx, "2026.7.1", host) {
        loft::self_update::Plan::Available { to, url, .. } => {
            assert_eq!(to, "2026.7.2");
            assert!(url.contains(host), "must offer THIS host's bundle: {url}");
        }
        other => panic!("expected an available update, got {other:?}"),
    }

    // Non-vacuity: the same index must NOT claim a build for a triple we never publish.
    match loft::self_update::plan(&idx, "2026.7.1", "riscv64-unknown-linux-gnu") {
        loft::self_update::Plan::NoBuildForTarget { .. } => {}
        other => panic!("an unpublished triple must be reported, got {other:?}"),
    }
    let _ = fa::remove_dir_all(&dir);
}

/// Splicing a toolchain entry must never date the index EARLIER than it already is.
///
/// `updated` says when the index last changed, and a toolchain entry is normally
/// submitted well after the release it names -- 2026.8.0 was published on the 1st and
/// submitted on the 31st -- with libraries landing in between.  Writing the release's
/// own `published` stamp into `updated` therefore moved the index's date BACKWARDS,
/// behind packages it already carried.  Nothing in the client compares the field, so
/// this could only ever be caught by a reader of the diff; it was, on the first real
/// submission, which is one submission later than a test costs.
#[test]
fn splicing_the_toolchain_entry_never_moves_updated_backwards() {
    let dir = std::env::temp_dir().join("loft_splice_updated_test");
    let _ = fa::remove_dir_all(&dir);
    let version = "2026.7.2";
    // `--published` is older than the index's own `updated`, the ordering that bit.
    let _ = generated_toolchain_entry(&dir, version);

    let index = dir.join("index.json");
    let newer = "2026-08-29T22:22:51Z";
    fa::write(
        &index,
        format!("{{\"schema_version\": 1, \"updated\": \"{newer}\", \"packages\": {{}}}}"),
    )
    .unwrap();

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = loft::platform::process::harness_command("python3")
        .arg(root.join("scripts/gen-toolchain-entry.py"))
        .args(["--version", version])
        .arg("--dir")
        .arg(&dir)
        .args(["--published", "2026-07-31T00:00:00Z"])
        .arg("--splice-into")
        .arg(&index)
        .current_dir(&root)
        .output()
        .expect("run gen-toolchain-entry.py --splice-into");
    assert!(
        out.status.success(),
        "splice failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Read it back through the parser that consumes the index in production, so the
    // test cannot pass on a file the client would reject.
    let text = fa::read_to_string(&index).unwrap();
    let spliced = loft::registry_index::parse_index(&text).expect("spliced index parses");
    assert_eq!(
        spliced.updated, newer,
        "a splice dated the index earlier than the packages it already held"
    );
    // Non-vacuity: the entry really did land, so the assertion above is about the
    // timestamp and not about a splice that quietly did nothing.
    let pkg = spliced
        .packages
        .get("loft")
        .expect("the entry must actually be spliced in");
    assert!(
        pkg.versions.contains_key(version),
        "the spliced entry must carry {version}"
    );
    let _ = fa::remove_dir_all(&dir);
}
