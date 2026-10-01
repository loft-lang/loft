// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The publish path may not mint an index the registry's own validation rejects.
//!
//! `tools/validate.py` gate 1 requires every package to carry a non-empty `categories`,
//! and `registry_maintain.sh` used to seed a brand-new package with `[]` — so every
//! package first published after that gate landed (2026-06-19) went in unmergeable.
//! `zttext` and `fixstep` are the two that did, and the red tick lands on the NEXT
//! submission PR, where nobody but the key holder can clear it.
//!
//! Two guards, and both are asserted here:
//!
//!   * the **door** — the own-lib fold refuses a package the index has never seen when its
//!     `loft.toml` declares no `[package] categories`;
//!   * the **chokepoint** — `registry_schema_gate.sh` refuses an index that fails gate 1,
//!     so a hand edit or a second publish path cannot get one signed either.
//!
//! Both cases run the code out of the SCRIPTS themselves rather than a copy of their rules:
//! the fold's Python body is lifted from `registry_maintain.sh`, and the gate helper is
//! invoked directly.  A restatement here would be a second list that drifts from the first.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_regcat_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Lift the own-lib index fold out of `registry_maintain.sh`: everything from its argument
/// unpacking to the heredoc terminator.  Reading the script keeps this test honest — an
/// edit that changes the rule changes what runs here too.
fn fold_source() -> String {
    let script = std::fs::read_to_string(repo_root().join("scripts/registry_maintain.sh"))
        .expect("registry_maintain.sh readable");
    let start = script
        .find("index_path, name, ver, entry_path, homepage, desc, desc_src, cats_raw")
        .expect("the fold's argument line — did registry_maintain.sh change shape?");
    let rest = &script[start..];
    let end = rest.find("\nEOF\n").expect("the fold heredoc terminator");
    format!("import json, sys\n{}", &rest[..end])
}

struct Fold {
    dir: PathBuf,
    script: PathBuf,
    index: PathBuf,
}

impl Fold {
    fn new(tag: &str) -> Self {
        let dir = work_dir(tag);
        let script = dir.join("fold.py");
        std::fs::write(&script, fold_source()).expect("write fold");
        let index = dir.join("index.json");
        std::fs::write(
            &index,
            r#"{"schema_version": 1, "updated": "2026-08-01T00:00:00Z", "packages": {
                 "already": {"description": "an existing library",
                             "homepage": "https://example.invalid",
                             "categories": ["text"], "yanked": [], "versions": {}},
                 "withdeps": {"description": "a library with a registry dependency",
                              "homepage": "https://example.invalid",
                              "categories": ["game"], "yanked": [],
                              "versions": {"0.1.0": {"url": "u", "sha256": "s",
                                                     "deps": {"hex_field": ">=0.1"}}}}}}"#,
        )
        .expect("write index");
        Fold { dir, script, index }
    }

    /// Run the fold for one publish.  Returns (success, combined output).
    fn publish(&self, name: &str, ver: &str, cats: &str) -> (bool, String) {
        self.publish_with_stderr(name, ver, cats, None)
    }

    /// The same, with a `loft publish` stderr log for the fold to read.  That log is where
    /// `[publish] deps-incomplete:` names the dependencies the package's SOURCE uses, which
    /// the fold reconciles against the entry — so a cell about deps supplies one and every
    /// other cell passes a path that does not exist, the ordinary case.
    fn publish_with_stderr(
        &self,
        name: &str,
        ver: &str,
        cats: &str,
        stderr: Option<&str>,
    ) -> (bool, String) {
        self.publish_described(name, ver, cats, stderr, "a real one-line description")
    }

    /// The same, with the manifest `description` the publish carries.
    fn publish_described(
        &self,
        name: &str,
        ver: &str,
        cats: &str,
        stderr: Option<&str>,
        desc: &str,
    ) -> (bool, String) {
        let entry = self.dir.join(format!("entry_{name}_{ver}.json"));
        std::fs::write(
            &entry,
            format!(
                r#""{ver}": {{"url": "u", "sha256": "s", "published": "2026-08-21T00:00:00Z"}}"#
            ),
        )
        .expect("write entry");
        let err_path = self.dir.join(format!("pub_{name}.err"));
        match stderr {
            Some(text) => std::fs::write(&err_path, text).expect("write stderr log"),
            None => {
                let _ = std::fs::remove_file(&err_path);
            }
        }
        let out = Command::new("python3")
            .arg(&self.script)
            .arg(&self.index)
            .arg(name)
            .arg(ver)
            .arg(&entry)
            .arg("https://example.invalid/pkg")
            .arg(desc)
            .arg("manifest")
            .arg(cats)
            .arg(&err_path)
            .arg(self.meta_mark(name))
            .output()
            .expect("run fold");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }

    /// Where the fold marks a package whose metadata it refreshed.
    fn meta_mark(&self, name: &str) -> PathBuf {
        self.dir.join(format!("meta_{name}"))
    }

    fn categories_of(&self, name: &str) -> Vec<String> {
        let raw = std::fs::read_to_string(&self.index).expect("read index");
        // Deliberately crude: the index is a fixture, and pulling in a JSON dependency to
        // read one array would be more machinery than the assertion is worth.
        let key = format!("\"{name}\"");
        let at = raw
            .find(&key)
            .unwrap_or_else(|| panic!("{name} absent from the index"));
        let cats_at = raw[at..].find("\"categories\"").expect("categories key") + at;
        let open = raw[cats_at..].find('[').expect("open bracket") + cats_at;
        let close = raw[open..].find(']').expect("close bracket") + open;
        raw[open + 1..close]
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// A package the index has never seen, published from a manifest with no `categories`, is
/// REFUSED — and the message names the manifest key to add, since that is the whole fix.
#[test]
fn a_new_package_without_categories_is_refused() {
    let fold = Fold::new("new_none");
    let (ok, out) = fold.publish("newpkg", "0.1.0", "");
    assert!(
        !ok,
        "the fold accepted a new package with no categories:\n{out}"
    );
    assert!(
        out.contains("categories"),
        "the refusal must name the field:\n{out}"
    );
    assert!(
        !std::fs::read_to_string(&fold.index)
            .expect("read index")
            .contains("newpkg"),
        "a refused publish must not leave the package in the index"
    );
}

/// The same publish with categories declared lands them — proving the refusal above is the
/// missing field and not the fixture.  A guard that cannot pass proves nothing.
#[test]
fn a_new_package_with_categories_is_accepted() {
    let fold = Fold::new("new_some");
    let (ok, out) = fold.publish("newpkg", "0.1.0", r#"["animation", "game"]"#);
    assert!(ok, "the fold refused a well-formed new package:\n{out}");
    assert_eq!(fold.categories_of("newpkg"), vec!["animation", "game"]);
}

/// An EXISTING package publishes from a manifest that declares none — accepted, and the
/// index's curated list survives.  This is why the refusal is scoped to new packages: 34 of
/// the 36 packages in the live index have categories that live only there.
#[test]
fn an_existing_packages_curated_categories_survive_a_silent_manifest() {
    let fold = Fold::new("existing_silent");
    let (ok, out) = fold.publish("already", "0.2.0", "");
    assert!(ok, "the fold refused an existing package:\n{out}");
    assert_eq!(fold.categories_of("already"), vec!["text"]);
}

/// A manifest that DOES declare them is authoritative and refreshes the index — the same
/// rule `description` follows, so a correction propagates on the next publish.
#[test]
fn a_declaring_manifest_refreshes_an_existing_list() {
    let fold = Fold::new("existing_refresh");
    let (ok, _) = fold.publish("already", "0.2.0", r#"["text", "editor"]"#);
    assert!(ok);
    assert_eq!(fold.categories_of("already"), vec!["text", "editor"]);
}

/// `deps` is a CLAIM, and `loft publish` can only read it from `loft.toml` — which a
/// multi-package repo deliberately leaves empty, because declaring registry deps there
/// resolves them from the registry instead of the `--lib` path.  So an empty `deps` in a
/// fresh entry means "not stated here", and pasting it verbatim publishes a version that
/// resolves none of its dependencies.  The previous entry is where those versions were
/// stated, so the fold carries them forward.
///
/// Measured on the real thing: hex_edge 0.2.0 would have published with `"deps": {}` and
/// lost its hex_field dependency, which `loft install` reads from the index and nowhere
/// else.  Only a JSON parse error in the entry stopped it.
#[test]
fn an_empty_deps_carries_the_previous_versions_forward() {
    let fold = Fold::new("deps_carry");
    let (ok, out) = fold.publish("withdeps", "0.2.0", "");
    assert!(ok, "the fold refused an existing package:\n{out}");
    let index = std::fs::read_to_string(&fold.index).expect("read index");
    assert_eq!(
        index.matches("hex_field").count(),
        2,
        "0.2.0 must carry 0.1.0's dependency forward; index now:\n{index}"
    );
}

/// …and a dependency named by NEITHER the entry nor a previous one is a refusal, not a
/// note.  `[publish] deps-incomplete:` on `loft publish`'s stderr names what the source
/// actually uses; a package new to the index has no earlier entry to inherit from, so
/// there is nowhere left for the versions to come from.
#[test]
fn a_source_dependency_in_neither_place_is_refused() {
    let fold = Fold::new("deps_missing");
    let (ok, out) = fold.publish_with_stderr(
        "newpkg",
        "0.1.0",
        r#"["game"]"#,
        Some("[publish] deps-incomplete: hex_field, hex_grid\n"),
    );
    assert!(
        !ok,
        "the fold accepted an entry with unresolvable deps:\n{out}"
    );
    assert!(
        out.contains("hex_field") && out.contains("hex_grid"),
        "the refusal must name what the source uses:\n{out}"
    );
    assert!(
        !std::fs::read_to_string(&fold.index)
            .expect("read index")
            .contains("newpkg"),
        "a refused publish must not leave the package in the index"
    );
}

/// Malformed input is refused rather than silently read as "none declared" — an empty
/// string element or a value that is not a JSON list.
#[test]
fn malformed_categories_are_refused() {
    let fold = Fold::new("malformed");
    for cats in [r#"[""]"#, "not-json", r#"{"a": 1}"#] {
        let (ok, out) = fold.publish("newpkg", "0.1.0", cats);
        assert!(!ok, "the fold accepted malformed categories {cats}:\n{out}");
    }
}

// ── the chokepoint ────────────────────────────────────────────────────────────

/// Build a registry checkout fixture: an index plus a `tools/validate.py` whose
/// `gate_schema` accepts or rejects.  A stub rather than the real validator because the real
/// one lives in `loft-lang/registry`; what is under test here is the WIRING — that a
/// rejection refuses and an absent validator refuses too.  The rules themselves are the
/// registry's, which is exactly why the gate reads them from the checkout.
fn registry_fixture(tag: &str, validator: Option<&str>) -> PathBuf {
    let dir = work_dir(tag);
    std::fs::write(
        dir.join("index.json"),
        r#"{"schema_version": 1, "packages": {}}"#,
    )
    .expect("write index");
    if let Some(body) = validator {
        std::fs::create_dir_all(dir.join("tools")).expect("tools dir");
        std::fs::write(dir.join("tools/validate.py"), body).expect("write validator");
    }
    dir
}

/// The bash that runs a POSIX script on Windows — Git Bash, never `bash` off `PATH`.
///
/// A bare `bash` resolves to `C:\Windows\System32\bash.exe`, the **WSL launcher**, because
/// System32 comes first on the runner's `PATH`.  With no distribution installed it prints
/// "Windows Subsystem for Linux has no installed distributions" (in UTF-16, which is why the
/// nightly log showed it letter-spaced) and exits non-zero WITHOUT running the script.  All
/// three gate tests then failed for one reason that had nothing to do with the gate: the
/// accepting fixture looked refused, and the two refusals carried the wrong message.
///
/// Deliberately falls back to plain `bash` rather than skipping when Git Bash is absent: a
/// skipped gate test looks exactly like a passing one, which is the very property
/// `a_missing_validator_refuses_rather_than_skips` exists to deny.
#[cfg(windows)]
fn windows_bash() -> PathBuf {
    // Beside `git.exe`: `<root>/cmd/git.exe` and `<root>/bin/bash.exe` ship together, so
    // finding one locates the other whatever drive Git was installed on.
    if let Ok(out) = Command::new("where").arg("git").output()
        && let Some(first) = String::from_utf8_lossy(&out.stdout).lines().next()
    {
        let git = PathBuf::from(first.trim());
        if let Some(root) = git.parent().and_then(|p| p.parent()) {
            let candidate = root.join("bin").join("bash.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    for root in ["C:\\Program Files\\Git", "C:\\Program Files (x86)\\Git"] {
        let candidate = Path::new(root).join("bin").join("bash.exe");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("bash")
}

fn run_gate(dir: &Path) -> (i32, String) {
    let script = repo_root().join("scripts/registry_schema_gate.sh");
    // Windows `CreateProcess` has no shebang handling, so handing it a `.sh` fails outright
    // with `%1 is not a valid Win32 application` — not a gate that refused, a gate that never
    // ran.  The interpreter has to be named there; on unix the shebang still picks it.
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new(windows_bash());
        c.arg(&script);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = Command::new(&script);
    let out = cmd.arg(dir).output().expect("run registry_schema_gate.sh");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

/// A validator that rejects makes the gate refuse, and its message reaches the operator —
/// the reason a rejection is worth having at all.
#[test]
fn the_schema_gate_refuses_what_the_validator_rejects() {
    let dir = registry_fixture(
        "gate_reject",
        Some(
            "import sys\ndef gate_schema(idx):\n    print('::error::deliberate rejection')\n    sys.exit(1)\n",
        ),
    );
    let (code, out) = run_gate(&dir);
    assert_ne!(code, 0, "the gate passed a rejected index:\n{out}");
    assert!(out.contains("deliberate rejection"), "message lost:\n{out}");
}

/// The same fixture with an accepting validator passes — so the refusal above is the
/// verdict and not the plumbing.
#[test]
fn the_schema_gate_passes_what_the_validator_accepts() {
    let dir = registry_fixture("gate_accept", Some("def gate_schema(idx):\n    pass\n"));
    let (code, out) = run_gate(&dir);
    assert_eq!(code, 0, "the gate refused an accepted index:\n{out}");
}

/// No validator in the checkout is a REFUSAL, not a skip.  A gate that skips looks exactly
/// like a gate that passes, and this one stands in front of the signing key.
#[test]
fn a_missing_validator_refuses_rather_than_skips() {
    let dir = registry_fixture("gate_absent", None);
    let (code, out) = run_gate(&dir);
    assert_ne!(code, 0, "an unchecked index was accepted:\n{out}");
    assert!(out.contains("refusing"), "the refusal must say so:\n{out}");
}

/// A publish that rewrites an EXISTING package's description or categories marks it, so
/// `--only` can hand the signer `--expect-meta <pkg>` beside its `--expect <pkg>@<ver>` —
/// without the mark the signer refuses the run's own metadata refresh.  A new package and
/// an unchanged one are not marked: the version is the whole change there.
#[test]
fn a_refreshed_description_or_list_is_marked_for_the_signer() {
    let fold = Fold::new("meta_mark");
    let (ok, out) = fold.publish_described("already", "0.2.0", "", None, "an existing library");
    assert!(ok, "{out}");
    assert!(
        !fold.meta_mark("already").exists(),
        "unchanged metadata is not marked"
    );
    let (ok, out) = fold.publish_described("already", "0.3.0", "", None, "a corrected line");
    assert!(ok, "{out}");
    assert!(
        fold.meta_mark("already").exists(),
        "a refreshed description is marked"
    );
    let (ok, out) = fold.publish("fresh", "0.1.0", r#"["text"]"#);
    assert!(ok, "{out}");
    assert!(
        !fold.meta_mark("fresh").exists(),
        "a new package is not marked"
    );
}
