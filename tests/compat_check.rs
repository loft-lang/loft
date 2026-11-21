// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![cfg(feature = "registry")]

//! `loft compat check` compares a package with the releases the REGISTRY lists, and holds
//! each kind of break against its own floor.
//!
//! Two properties, both exit codes (the library CI and the publish script branch on them):
//!
//! - loft#2599 — the releases compared are the published ones, fetched when they are not in
//!   the install cache.  A check that reads the cache alone passes on a machine that never
//!   installed the package, which is every CI runner.
//! - loft#2598 — a reshaped record is a break of `data_compatible_with`, a changed surface
//!   one of `api_compatible_with`; raising the floor of the kind that broke declares it.
//!
//! Hermetic: `LOFT_HOME` is a per-test cache and `LOFT_REGISTRY_URL` a `file://` index whose
//! tarballs are built here, so the whole path runs — resolve, download, verify, unpack,
//! compare — with no socket.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

mod common;

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().unwrap()).expect("mkdir");
    fa::write(path, body).expect("write");
}

/// A package directory `dir` for `name` at `version`, with `source` as its one file.
fn package(dir: &Path, name: &str, version: &str, api: &str, data: &str, source: &str) {
    write(
        &dir.join("loft.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nloft = \">=0.8\"\n\
             api_compatible_with  = \"{api}\"\ndata_compatible_with = \"{data}\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n"
        ),
    );
    write(&dir.join("src").join(format!("{name}.loft")), source);
}

/// A scratch root for one test: `home` (the private cache), `served` (the registry) and
/// `tree` (the working tree under check).
struct Bench {
    root: PathBuf,
}

impl Bench {
    fn new(tag: &str) -> Bench {
        let root = std::env::temp_dir().join(format!("loft_compat_{tag}_{}", std::process::id()));
        let _ = fa::remove_dir_all(&root);
        fa::create_dir_all(root.join("home")).expect("mkdir");
        Bench { root }
    }

    fn tree(&self) -> PathBuf {
        self.root.join("tree")
    }

    /// Publish `name` 0.1.0 with `source` into the registry; answers the index URL.
    fn publish_first(&self, name: &str, source: &str) -> String {
        let src = self.root.join("first");
        package(&src, name, "0.1.0", "0.1.0", "0.1.0", source);
        let served = self.root.join("served");
        fa::create_dir_all(&served).expect("mkdir");
        let out = loft::package::package_create(&src, Some(&served)).expect("package_create");
        let index = served.join("index.json");
        write(
            &index,
            &format!(
                r#"{{"schema_version":1,"packages":{{"{name}":{{"description":"probe",
                   "versions":{{"0.1.0":{{"url":"{}","sha256":"{}","size":{},
                   "loft":">=0.8","published":"2026-08-18T00:00:00Z"}}}}}}}}}}"#,
                common::file_url(&out.tarball),
                out.sha256,
                out.size
            ),
        );
        common::file_url(&index)
    }

    /// `loft compat check [--full]` in the working tree: whether it passed, and what it said.
    fn check(&self, url: &str, full: bool) -> (bool, String) {
        let mut args = vec!["compat", "check"];
        if full {
            args.push("--full");
        }
        let home = self.root.join("home");
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args(&args)
            .env("LOFT_HOME", &home)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LOFT_REGISTRY_URL", url)
            .env("LOFT_TIMEOUT", "120")
            .current_dir(self.tree())
            .output()
            .expect("spawn loft");
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = fa::remove_dir_all(&self.root);
    }
}

const ONE: &str = "pub fn one(x: integer) -> integer { return x + 1; }\n";
const ONE_AND_TWO: &str = "pub fn one(x: integer) -> integer { return x + 1; }\n\
                           pub fn two(x: integer) -> integer { return x + 2; }\n";
const TWO_ONLY: &str = "pub fn two(x: integer) -> integer { return x + 2; }\n";

/// loft#2599 — a published release that is not in the install cache is fetched and compared:
/// removing a function it had fails the sampled check and the full one, on an empty cache.
#[test]
fn a_break_against_a_published_release_is_found_on_an_empty_cache() {
    let b = Bench::new("cold_break");
    let url = b.publish_first("coldpkg", ONE);
    package(&b.tree(), "coldpkg", "0.2.0", "0.1.0", "0.1.0", TWO_ONLY);

    let (ok, said) = b.check(&url, false);
    assert!(
        !ok,
        "the sampled check must fail on a removed function:\n{said}"
    );
    assert!(
        said.contains("0.1.0: API BREAK"),
        "it names the release it compared against:\n{said}"
    );
    let (ok, said) = b.check(&url, true);
    assert!(
        !ok,
        "the full check must fail on a removed function:\n{said}"
    );
    assert!(
        said.contains("0.1.0: BREAK (API)") && said.contains("`api_compatible_with`"),
        "it names the release, the kind of break and the floor to raise:\n{said}"
    );
}

/// The control of the test above: the same registry and empty cache, a tree that only adds.
/// It passes, and it says it looked at the published release — a pass over no release is
/// what loft#2599 was.
#[test]
fn an_addition_passes_and_says_which_release_it_was_compared_with() {
    let b = Bench::new("cold_ok");
    let url = b.publish_first("coldpkg", ONE);
    package(&b.tree(), "coldpkg", "0.2.0", "0.1.0", "0.1.0", ONE_AND_TWO);

    let (ok, said) = b.check(&url, false);
    assert!(ok, "an addition is a drop-in:\n{said}");
    assert!(
        said.contains("1 of 1 release(s) sampled") && said.contains("0.1.0: api ok, data ok"),
        "the pass names the release it compared against:\n{said}"
    );
    let (ok, said) = b.check(&url, true);
    assert!(ok, "an addition is a drop-in for the full check:\n{said}");
    assert!(
        said.contains("whole claim verified") && said.contains("(1 release(s))"),
        "the full pass counts the release it verified:\n{said}"
    );
}

/// loft#2599 — with the registry unreadable and nothing cached, which releases exist is not
/// known: the check fails and says so, where it used to pass with "nothing to compare".
#[test]
fn an_unreadable_registry_with_an_empty_cache_is_a_failure() {
    let b = Bench::new("no_registry");
    package(&b.tree(), "coldpkg", "0.2.0", "0.1.0", "0.1.0", ONE);
    let nowhere = common::file_url(&b.root.join("served").join("missing.json"));

    for full in [false, true] {
        let (ok, said) = b.check(&nowhere, full);
        assert!(
            !ok,
            "full={full}: a check that compared nothing must not pass:\n{said}"
        );
        assert!(
            said.contains("the registry could not be read")
                && said.contains("nothing was verified"),
            "full={full}: it says why nothing was verified:\n{said}"
        );
    }
}

/// A package the registry does not list and the cache does not hold has no earlier release
/// to keep a promise to: that is a pass, and the message says the registry was asked.
#[test]
fn a_package_never_published_passes() {
    let b = Bench::new("unpublished");
    let url = b.publish_first("otherpkg", ONE);
    package(&b.tree(), "newpkg", "0.1.0", "0.1.0", "0.1.0", ONE);

    let (ok, said) = b.check(&url, false);
    assert!(ok, "a first release has nothing to break:\n{said}");
    assert!(
        said.contains("the registry lists no other release"),
        "it says the registry was read:\n{said}"
    );
}

const INNER_A: &str = "struct Inner { a: integer }\n\
                       pub fn make(n: integer) -> integer { i = Inner { a: n }; i.a }\n";
const INNER_AB: &str = "struct Inner { a: integer, b: integer }\n\
                        pub fn make(n: integer) -> integer { i = Inner { a: n, b: 0 }; i.a + i.b }\n";

/// loft#2598 — a record reshaped behind an unchanged surface is a break of the DATA floor:
/// at the old data floor it fails and names `data_compatible_with`; with that floor raised
/// and the API floor left where it was, it is a declared break and the check passes.
#[test]
fn a_layout_break_is_held_against_the_data_floor() {
    let b = Bench::new("data_floor");
    let url = b.publish_first("laypkg", INNER_A);

    package(&b.tree(), "laypkg", "0.2.0", "0.1.0", "0.1.0", INNER_AB);
    let (ok, said) = b.check(&url, true);
    assert!(
        !ok,
        "a reshaped record at the old data floor must fail:\n{said}"
    );
    assert!(
        said.contains("0.1.0: BREAK (layout)")
            && said.contains("raise `data_compatible_with` past them"),
        "it is a layout break, cured by the data floor alone:\n{said}"
    );
    let (ok, said) = b.check(&url, false);
    assert!(!ok, "the sampled check fails on it too:\n{said}");
    assert!(
        said.contains("0.1.0: api ok, DATA BREAK") && said.contains("raise `data_compatible_with`"),
        "the sampled check names the data floor:\n{said}"
    );

    package(&b.tree(), "laypkg", "0.2.0", "0.1.0", "0.2.0", INNER_AB);
    let (ok, said) = b.check(&url, true);
    assert!(ok, "the data floor raised declares the break:\n{said}");
    assert!(
        said.contains("0.1.0: DECLARED BREAK (below the floor)"),
        "the declared break is still reported:\n{said}"
    );
    let (ok, said) = b.check(&url, false);
    assert!(ok, "the sampled check accepts the declared break:\n{said}");
    assert!(
        said.contains("DECLARED BREAK") && said.contains("data 0.2.0"),
        "and says which floor was raised:\n{said}"
    );
}

/// A record that gained a public field is an API break AND a reshaped record.  The API floor
/// raised past it declares both: the package may keep its data floor where it was, which says
/// the files it writes are still read (hex_body 0.2.0 did exactly this, and a format of a
/// package's own is not its records' layout).
#[test]
fn a_reshaped_public_record_is_declared_by_the_api_floor_alone() {
    let b = Bench::new("api_declares_layout");
    let url = b.publish_first(
        "recpkg",
        "pub struct Rig { pub a: integer }\npub fn rig() -> Rig { Rig { a: 1 } }\n",
    );
    let wider = "pub struct Rig { pub a: integer, pub b: integer }\n\
                 pub fn rig() -> Rig { Rig { a: 1, b: 2 } }\n";

    package(&b.tree(), "recpkg", "0.2.0", "0.2.0", "0.1.0", wider);
    for full in [false, true] {
        let (ok, said) = b.check(&url, full);
        assert!(
            ok,
            "full={full}: the API floor raised declares the reshaped record:\n{said}"
        );
        assert!(
            said.contains("DECLARED BREAK"),
            "full={full}: and the break is still reported:\n{said}"
        );
    }

    // The control: with neither floor raised it is the unclaimed break.
    package(&b.tree(), "recpkg", "0.2.0", "0.1.0", "0.1.0", wider);
    let (ok, said) = b.check(&url, true);
    assert!(
        !ok,
        "neither floor raised: a break nobody declared:\n{said}"
    );
}

/// The other direction of loft#2598: raising the DATA floor does not declare an API break.
#[test]
fn an_api_break_is_not_declared_by_the_data_floor() {
    let b = Bench::new("api_floor");
    let url = b.publish_first("coldpkg", ONE);
    package(&b.tree(), "coldpkg", "0.2.0", "0.1.0", "0.2.0", TWO_ONLY);

    let (ok, said) = b.check(&url, true);
    assert!(
        !ok,
        "a removed function is an API break whatever the data floor:\n{said}"
    );
    assert!(
        said.contains("0.1.0: BREAK (API)")
            && said.contains("raise `api_compatible_with` past them"),
        "it names the API floor:\n{said}"
    );
}
