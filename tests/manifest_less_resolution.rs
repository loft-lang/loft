// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN143 — manifest-less resolution: a program may omit its dependency declaration.
//!
//! A bare script (no `loft.toml`, no sidecar lock) means *the newest release, re-decided
//! every run*. Today the ergonomics already work — `lib_path`'s last probe auto-installs —
//! but a run LEAVES SOMETHING BEHIND that changes what the next run resolves, which is the
//! defect this plan closes.
//!
//! **The invariant:** nothing a program produces by RUNNING may change which version a
//! later run resolves. A version is fixed only by an explicit act (`loft install`,
//! `loft update`, `loft pin`), and that act writes exactly one declaration, beside the
//! thing it governs.
//!
//! Hermetic throughout: `LOFT_HOME` points at a per-test cache and `LOFT_REGISTRY_URL` at
//! a path that cannot be fetched (or a `file://` fixture registry), so every cell measures
//! resolution rather than the network.
//!
//! ⚠ **What these cells cannot measure, and why.** Arc A makes the auto-install path
//! refuse an index without a VALID signature, and a test cannot forge one — the trust root
//! is compiled in. So the cells reach resolution through the extracted cache and through
//! the real `loft install` CLI (which keeps its own `--allow-unsigned` default), never
//! through a bare `use` that installs. The write half of the invariant — *a run leaves no
//! lockfile behind* — is therefore pinned by the `lock_write_target` unit tests in
//! `src/resolution_scope.rs`, plus a measurement against the real registry recorded in the
//! arc C2 commit: before it, one run of `use arguments;` left `./loft.lock`; after it, the
//! directory holds only the script.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/mod.rs"]
mod common;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// A private `~/.loft` whose registry cache holds `index.json` for one package, with the
/// signature beside it decided by `sig`.
///
/// The index is written FRESH, so `index_stale` is false and no fetch is attempted — the
/// cached-index branch is the one under test, and it verifies like every other branch.
fn fake_registry(tag: &str, sig: Option<&str>) -> PathBuf {
    let home = std::env::temp_dir().join(format!("loft_pln143_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let cache = home.join(".loft/registry");
    write(
        &cache.join("index.json"),
        r#"{"schema_version":1,"packages":{"probepkg":{"description":"probe",
           "versions":[{"semver":"0.1.0","url":"http://127.0.0.1:1/probepkg-0.1.0.tar.gz",
           "sha256":"0000000000000000000000000000000000000000000000000000000000000000",
           "deps":{}}]}}}"#,
    );
    if let Some(s) = sig {
        write(&cache.join("index.json.sig"), s);
    }
    home
}

/// Run `script` from `dir` against a private registry cache.
fn run_in(home: &Path, dir: &Path, script: &str) -> String {
    run_env(home, dir, script, &[])
}

/// [`run_in`] plus `env` — `LOFT_OFFLINE=1` for the cells that must resolve with no
/// registry at all.
fn run_env(home: &Path, dir: &Path, script: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(loft_bin());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .args(["--interpret", script])
        .env("LOFT_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        // Unreachable: a cell that reached the real registry would not be measuring
        // resolution, and would fail differently on a machine with no network.
        .env("LOFT_REGISTRY_URL", "http://127.0.0.1:1/index.json")
        .env("LOFT_TIMEOUT", "90")
        .current_dir(dir)
        .output()
        .expect("spawn loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// @PLN143 arc A — the auto-install path does not waive the index signature.
///
/// `allow_unsigned` was `true` there, mirroring `loft install`. That is defensible while
/// auto-install is a fallback nobody relies on; this plan makes it the DEFAULT path a bare
/// `use` takes, and widening the blast radius while leaving the bootstrap window open must
/// not be two commits in that order.
///
/// An INVALID signature was already a hard failure everywhere — what this pins is the
/// MISSING one, which is the case `allow_unsigned` actually waived.
///
/// ⚠ The assertion keys on the REFUSAL MESSAGE, not on "the run failed". Both
/// configurations fail here — the fixture's tarball is unreachable by construction — so
/// "it did not resolve" cannot tell them apart, and a first version of this test that
/// asserted only that passed with arc A REVERTED. The two were measured:
///
///   with arc A:    `registry index signature is malformed or missing; pass --allow-unsigned`
///   without arc A: `no version of `probepkg` satisfies constraint `*``
///
/// i.e. without the arc the run gets PAST the signature and fails later, on the index's
/// own content. That difference is the whole claim, so it is what is asserted.
#[test]
fn arc_a_auto_install_refuses_an_unsigned_index() {
    let home = fake_registry("unsigned", None);
    let dir = home.join("proj");
    write(
        &dir.join("s.loft"),
        "use probepkg;\nfn main() { println(\"resolved\"); }\n",
    );
    let all = run_in(&home, &dir, "s.loft");
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("signature is malformed or missing"),
        "an unsigned index must be REFUSED by the signature gate on the auto-install \
         path — reaching any later failure means the gate was waived (@PLN143 arc A):\n{all}"
    );
    assert!(
        !all.contains("resolved"),
        "and nothing may resolve through it:\n{all}"
    );
}

/// The control that makes the cell above a comparison rather than an anecdote: the same
/// cache with a signature PRESENT must get past the signature gate and fail later, on the
/// index content instead. If this ever starts reporting a signature refusal, the gate has
/// begun rejecting something it was never meant to.
#[test]
fn arc_a_a_signed_index_gets_past_the_signature_gate() {
    // Malformed rather than valid: `verify_or_explain` treats malformed and missing as
    // the same waivable class, so this cell isolates PRESENCE — which is all it needs to
    // show the failure moving.
    let home = fake_registry("signed", Some("not-a-real-signature"));
    let dir = home.join("proj");
    write(
        &dir.join("s.loft"),
        "use probepkg;\nfn main() { println(\"resolved\"); }\n",
    );
    let all = run_in(&home, &dir, "s.loft");
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        !all.contains("resolved"),
        "the fixture's package cannot be downloaded, so nothing should resolve:\n{all}"
    );
}

// ── arc C1: a bare script falls back to the newest version already cached ──────────

/// A private `~/.loft` with no index at all — the cells below resolve from the extracted
/// cache, which is what a bare script has to fall back on when the registry cannot be
/// reached.
fn empty_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("loft_pln143_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".loft/registry")).expect("mkdir");
    home
}

/// Extract a package into `home`'s cache by hand — exactly the shape `install_one`
/// leaves behind. Its `probe_id()` answers `<name>-<version>`, so the RESOLVED VERSION is
/// the value the script prints: a cell cannot pass by resolving the wrong copy.
fn cache_pkg(home: &Path, name: &str, version: &str, loft_req: &str) {
    let dir = home
        .join(".loft/registry")
        .join(format!("{name}-{version}"));
    write(
        &dir.join("loft.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nloft = \"{loft_req}\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n"
        ),
    );
    write(
        &dir.join("src").join(format!("{name}.loft")),
        &format!("pub fn probe_id() -> text {{ return \"{name}-{version}\"; }}\n"),
    );
}

/// A `loft.lock` pinning one version of `probepkg`. The url/sha are unreachable by
/// construction — a cell that fetched would be measuring the network, not resolution.
fn pin_lock(version: &str) -> String {
    format!(
        "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"{version}\"\n\
         url = \"http://127.0.0.1:1/probepkg-{version}.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
         source = \"registry\"\n"
    )
}

/// A bare script that prints which version of `probepkg` it resolved.
fn probe_script(dir: &Path) {
    write(
        &dir.join("s.loft"),
        "use probepkg;\nfn main() { println(probe_id()); }\n",
    );
}

/// @PLN143 arc C1 — offline, bare script, package already in the cache.
///
/// This answered *"Library 'probepkg' not found — searched lib/, lib_dirs, and sibling
/// packages"* in a directory holding two extracted copies of it: the least true message
/// available. The stray cwd `loft.lock` a first run wrote is what covered this case
/// before, which is why the fallback has to land WITH the deletion of that leg and not
/// after it.
#[test]
fn arc_c1_offline_resolves_the_newest_cached_version() {
    let home = empty_home("c1_newest");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    let dir = home.join("proj");
    probe_script(&dir);
    let all = run_env(&home, &dir, "s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.2.0"),
        "the newest cached copy must resolve — 0.1.0 or a failure means the fallback \
         did not fire (@PLN143 arc C1):\n{all}"
    );
}

/// The two ways "newest" can be got wrong, in one cell because they compose: `0.10.0` is
/// newer than `0.2.0` only under semver (string order says the opposite), and a
/// prerelease is not a release — `find_best_version` skips one unless asked, and a
/// fallback is not the place to opt someone in.
#[test]
fn arc_c1_newest_is_semver_and_skips_a_prerelease() {
    let home = empty_home("c1_semver");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.10.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.11.0-beta.1", ">=0.8");
    let dir = home.join("proj");
    probe_script(&dir);
    let all = run_env(&home, &dir, "s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.10.0"),
        "0.10.0 is the newest RELEASE: lexicographic order would pick 0.2.0, and taking \
         the beta would install a prerelease nobody asked for:\n{all}"
    );
}

/// A cached copy this loft cannot load must not hide the older one that it can.
///
/// The loader rejects an unsatisfied `loft` requirement FATALLY, so picking the newest
/// directory and letting the load decide would turn a working fallback into a hard error
/// — the filter therefore asks the same `manifest::check_version` the loader asks, before
/// choosing.
#[test]
fn arc_c1_a_version_this_loft_cannot_load_is_skipped() {
    let home = empty_home("c1_floor");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.3.0", ">=9999");
    let dir = home.join("proj");
    probe_script(&dir);
    let all = run_env(&home, &dir, "s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.2.0"),
        "0.3.0 requires loft >=9999, so the newest LOADABLE copy is 0.2.0:\n{all}"
    );
    assert!(
        !all.contains("requires loft"),
        "and it is skipped silently — reaching the loader's fatal means the filter ran \
         too late:\n{all}"
    );
}

/// The boundary, and the whole rule: the fallback never answers PAST a declaration.
///
/// A package's manifest may say `^0.1` and a pinned script names an exact version;
/// picking "the newest cached" there would answer past a declaration whose entire purpose
/// is to be honoured.  So a declared scope takes only a cached copy its declaration allows
/// (loft#1687 — see `offline_a_cached_librarys_dependency_resolves_under_its_declared_range`),
/// and where none does — here, `^0.1` against a cache holding only 0.2.0 — it fails, naming
/// the range and what the cache holds.
#[test]
fn arc_c1_a_declared_scope_does_not_take_the_newest_cached() {
    let home = empty_home("c1_boundary");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    let pkg = home.join("proj");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n\n[dependencies]\nprobepkg = \"^0.1\"\n",
    );
    // A lockfile that resolves nothing for `probepkg`: the declaration is in force and
    // unsatisfied, which is exactly the case that must not fall back.
    write(&pkg.join("loft.lock"), "schema_version = 1\n");
    probe_script(&pkg.join("src"));
    let all = run_env(&home, &pkg, "src/s.loft", &[("LOFT_OFFLINE", "1")]);
    // The control that keeps this cell from passing on a broken fixture: the SAME cache
    // and the SAME script, one directory over with nothing declared, must resolve.
    let bare = home.join("bare");
    probe_script(&bare);
    let bare_out = run_env(&home, &bare, "s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        bare_out.contains("probepkg-0.2.0"),
        "control: the cached copy IS resolvable, so a miss above would be about the \
         scope and not about the fixture:\n{bare_out}"
    );
    assert!(
        !all.contains("probepkg-0.2.0"),
        "a package scope declares its own constraint — the cache fallback must not \
         answer past it:\n{all}"
    );
    assert!(
        all.contains("no cached copy satisfies `^0.1`") && all.contains("cached: 0.2.0"),
        "the refusal names the unmet range and the cached copies (loft#1687):\n{all}"
    );
}

// ── arc D: `loft install` in a directory that is not a package declares one ────────

/// Build a one-package registry on disk and return its `file://` index URL.
///
/// `file://` rather than a fixture HTTP server: `http_get_bytes` serves both the index
/// and the tarball from it, so the whole install path runs — resolve, download, verify
/// sha256, extract — with no socket and no second copy of a server in this file.
fn file_registry(home: &Path, name: &str, version: &str) -> String {
    let src = home.join("src_pkg");
    write(
        &src.join("loft.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nloft = \">=0.8\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n"
        ),
    );
    write(
        &src.join("src").join(format!("{name}.loft")),
        &format!("pub fn probe_id() -> text {{ return \"{name}-{version}\"; }}\n"),
    );
    let served = home.join("served");
    std::fs::create_dir_all(&served).expect("mkdir");
    let out = loft::package::package_create(&src, Some(&served)).expect("package_create");
    let index = served.join("index.json");
    write(
        &index,
        &format!(
            r#"{{"schema_version":1,"packages":{{"{name}":{{"description":"probe",
               "versions":{{"{version}":{{"url":"{}","sha256":"{}","size":{},
               "loft":">=0.8","published":"2026-08-18T00:00:00Z"}}}}}}}}}}"#,
            common::file_url(&out.tarball),
            out.sha256,
            out.size
        ),
    );
    common::file_url(&index)
}

/// Run the `loft` CLI in `dir` against the private home + `url` registry.
fn cli(home: &Path, dir: &Path, url: &str, args: &[&str]) -> String {
    let out = Command::new(loft_bin())
        .args(args)
        .env("LOFT_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOFT_REGISTRY_URL", url)
        .env("LOFT_TIMEOUT", "120")
        .current_dir(dir)
        .output()
        .expect("spawn loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// @PLN143 arc D — `loft install <pkg>@<version>` outside a package writes the manifest
/// that makes its lock govern, and a later run honours the pin.
///
/// Without this the verb's pin was honoured only through the cwd `loft.lock` leg that C2
/// deletes: an explicit version, asked for by hand, silently ignored on the next run —
/// worse than the defect being fixed. So the install CREATES the declaration it needs,
/// which makes the directory a package and leaves the scope table to decide the rest.
///
/// The second half is the half that matters: a NEWER version is put in the cache before
/// the run, so "the pin held" cannot pass by there being nothing else to resolve to.
#[test]
fn arc_d_install_outside_a_package_declares_one_and_the_pin_holds() {
    let home = empty_home("d_declare");
    let url = file_registry(&home, "probepkg", "0.1.0");
    let dir = home.join("scratch");
    std::fs::create_dir_all(&dir).expect("mkdir");

    let installed = cli(&home, &dir, &url, &["install", "probepkg@0.1.0"]);
    let manifest = std::fs::read_to_string(dir.join("loft.toml")).unwrap_or_default();
    let lock = std::fs::read_to_string(dir.join("loft.lock")).unwrap_or_default();

    // A newer copy in the cache: the pin below has to beat something.
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    probe_script(&dir);
    let ran = run_env(&home, &dir, "s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        manifest.contains("[package]") && manifest.contains("probepkg = \"0.1.0\""),
        "install into a directory with no manifest must WRITE one, declaring what it \
         installed (@PLN143 arc D):\n{installed}\nloft.toml:\n{manifest}"
    );
    assert!(
        installed.contains("created loft.toml"),
        "and say so — a file appearing unannounced is the surprise this replaces:\n{installed}"
    );
    assert!(
        lock.contains("0.1.0"),
        "the lock still records the exact version:\n{lock}"
    );
    assert!(
        ran.contains("probepkg-0.1.0"),
        "the pin must govern the next run even with 0.2.0 sitting in the cache — that \
         is what the manifest is FOR:\n{ran}"
    );
}

// ── arc C2: the cwd is not a scope, and running declares nothing ───────────────────

/// @PLN143 arc C2 — a `loft.lock` in the directory you happen to be standing in does not
/// decide which version a program gets.
///
/// This was the defect that defeated the feature with the feature: a bare script took
/// "latest" exactly once, wrote `./loft.lock`, and was pinned by that file forever. The
/// value oracle is the version itself — 0.1.0 is what the stray lock names, 0.2.0 is what
/// "newest, re-decided every run" means.
#[test]
fn arc_c2_a_cwd_lockfile_does_not_pin_a_bare_script() {
    let home = empty_home("c2_cwd");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    let dir = home.join("proj");
    probe_script(&dir);
    write(
        &dir.join("loft.lock"),
        "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\
         url = \"http://127.0.0.1:1/probepkg-0.1.0.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
         source = \"registry\"\n",
    );
    let all = run_env(&home, &dir, "s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.2.0"),
        "nothing declares anything here — a lockfile in the cwd is not a declaration, \
         and 0.1.0 means the deleted leg is still deciding (@PLN143 arc C2):\n{all}"
    );
}

/// loft#991 — the same for a file that is INSIDE A PROJECT whose project has no lock.
///
/// The two cells above cover a BARE script, which belongs to no project. This is the row
/// they leave open, and it is the one that was filed: `probe_project_lockfile` finds the
/// project root, finds no `loft.lock` there, and returns — and before this arc the chain
/// then fell through to the cwd probe, so a stranger's pin governed a project that
/// declares its own dependency in `loft.toml`. Measured on `tuxedo-post-973` (the branch
/// without this arc): the same project resolved `random-0.2.1` or `random-0.3.1` purely
/// by which directory `loft` was invoked from, against `0.1.0` in its own manifest.
///
/// A stray `loft.lock` is easy to acquire without asking for one, which is what made this
/// reachable: before this arc, running any bare script that `use`d a registry package
/// wrote one into the cwd. Do that once in `$HOME` and every lock-less project built from
/// `$HOME` resolved through it.
///
/// ⚠ **What this cell asserts is the ABSENCE of the stranger's version, and the control
/// below is what stops that from being vacuous.** Offline, a lock-less project resolves
/// NOTHING — `probe_cache_newest` is `Bare`-scope only by design (a fallback takes the
/// newest cached copy, and only where nothing is declared can that violate no
/// constraint), so a scope that HAS a declaration refuses instead. That refusal is
/// pre-existing and unchanged by this arc — measured identical on both branches with no
/// lockfile anywhere. So the difference this cell measures is exactly the filed one:
/// `probepkg-0.1.0` against the defect, no version at all with the leg gone.
#[test]
fn arc_c2_a_cwd_lockfile_does_not_pin_a_lockless_project() {
    let home = empty_home("c2_project");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    let proj = home.join("proj");
    write(
        &proj.join("loft.toml"),
        "[package]\nname = \"lockless\"\nversion = \"0.1.0\"\n\n\
         [dependencies]\nprobepkg = \"0.2.0\"\n",
    );
    // The script sits under the project root — `probe_project_lockfile` finds that root
    // and no lock in it, which is exactly the state that used to fall through.
    probe_script(&proj.join("src"));
    let elsewhere = home.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    write(&elsewhere.join("loft.lock"), &pin_lock("0.1.0"));
    let all = run_env(
        &home,
        &elsewhere,
        proj.join("src/s.loft").to_str().expect("path"),
        &[("LOFT_OFFLINE", "1")],
    );
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        !all.contains("probepkg-0.1.0"),
        "a project's resolution belongs to the PROJECT, not to the directory loft was \
         invoked from — 0.1.0 is the invoking directory's lockfile deciding \
         (loft#991):\n{all}"
    );
}

/// The control for the cell above: the SAME fixture, with the project holding its own
/// lock, must resolve — otherwise "no version appeared" would pass whatever the reason.
///
/// It also pins the precedence the filed report found already working: the project's lock
/// outranks the invoking directory's, and the two name different versions here so neither
/// can be reached by accident.
#[test]
fn arc_c2_a_projects_own_lock_outranks_the_invoking_directorys() {
    let home = empty_home("c2_project_ctl");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    let proj = home.join("proj");
    write(
        &proj.join("loft.toml"),
        "[package]\nname = \"locked\"\nversion = \"0.1.0\"\n\n\
         [dependencies]\nprobepkg = \"0.2.0\"\n",
    );
    probe_script(&proj.join("src"));
    write(&proj.join("loft.lock"), &pin_lock("0.2.0"));
    let elsewhere = home.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    write(&elsewhere.join("loft.lock"), &pin_lock("0.1.0"));
    let all = run_env(
        &home,
        &elsewhere,
        proj.join("src/s.loft").to_str().expect("path"),
        &[("LOFT_OFFLINE", "1")],
    );
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.2.0"),
        "the project's own lock governs it from any directory (loft#991):\n{all}"
    );
}

/// The same script, run from two directories, resolves the same way.
///
/// Cwd-dependence is the other half of the same defect: one of these directories holds a
/// lock and the other does not, and before this arc that alone changed which version
/// loaded — from a file neither the script nor its author named.
#[test]
fn arc_c2_the_same_script_resolves_the_same_from_two_directories() {
    let home = empty_home("c2_two_dirs");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    let script_dir = home.join("script");
    probe_script(&script_dir);
    let script = script_dir.join("s.loft");
    let elsewhere = home.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    write(
        &elsewhere.join("loft.lock"),
        "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\
         url = \"http://127.0.0.1:1/probepkg-0.1.0.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
         source = \"registry\"\n",
    );

    let from_home = run_env(
        &home,
        &script_dir,
        &script.to_string_lossy(),
        &[("LOFT_OFFLINE", "1")],
    );
    let from_elsewhere = run_env(
        &home,
        &elsewhere,
        &script.to_string_lossy(),
        &[("LOFT_OFFLINE", "1")],
    );
    // Nothing may appear in either directory as a result of running.
    let left_behind: Vec<String> = [&script_dir, &elsewhere]
        .iter()
        .flat_map(|d| std::fs::read_dir(d).expect("read_dir"))
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n == "loft.lock" || n == "loft.toml")
        .collect();
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        from_home.contains("probepkg-0.2.0") && from_elsewhere.contains("probepkg-0.2.0"),
        "one script, one answer, wherever it is run from:\n{from_home}\n---\n{from_elsewhere}"
    );
    assert_eq!(
        left_behind,
        vec!["loft.lock".to_string()],
        "only the lock the fixture WROTE may be there: a run declares nothing"
    );
}

// ── arc E: a pin that has fallen behind says so, once ──────────────────────────────

/// Write the cached `index.json` these cells compare against — the map-keyed shape the
/// real index has, and the one `parse_index` reads.
fn cached_index(home: &Path, name: &str, newest: &str) {
    write(
        &home.join(".loft/registry/index.json"),
        &format!(
            r#"{{"schema_version":1,"packages":{{"{name}":{{"description":"probe",
               "versions":{{"{newest}":{{"url":"http://127.0.0.1:1/{name}-{newest}.tar.gz",
               "sha256":"0000000000000000000000000000000000000000000000000000000000000000",
               "size":1,"loft":">=0.8","published":"2026-08-18T00:00:00Z"}}}}}}}}}}"#
        ),
    );
}

/// A package whose lock pins `version`, with a script under `src/`.
fn pinned_package(home: &Path, version: &str) -> PathBuf {
    let pkg = home.join("proj");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n\n[dependencies]\nprobepkg = \"*\"\n",
    );
    write(
        &pkg.join("loft.lock"),
        &format!(
            "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"{version}\"\n\
             url = \"http://127.0.0.1:1/probepkg-{version}.tar.gz\"\n\
             sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
             source = \"registry\"\n"
        ),
    );
    probe_script(&pkg.join("src"));
    pkg
}

/// @PLN143 arc E — a lock has no expiry, so a pin holds forever and the program's own
/// code never says why.
///
/// `cbor` 0.1.3 turned canonical map encode from O(n³) to O(n²) with NO API change: a
/// consumer pinned at 0.1.2 keeps hanging and nothing it can read explains it. This is the
/// one line that does — said once, from the cached index, with no fetch added to the run.
#[test]
fn arc_e_a_pin_behind_the_registry_says_so_once() {
    let home = empty_home("e_behind");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cached_index(&home, "probepkg", "0.2.0");
    let pkg = pinned_package(&home, "0.1.0");
    let all = run_env(&home, &pkg, "src/s.loft", &[]);
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.1.0"),
        "the pin still governs — the notice reports, it does not resolve:\n{all}"
    );
    assert!(
        all.contains("probepkg 0.1.0 is pinned; 0.2.0 is the newest release"),
        "a governing pin behind the cached index must say so (@PLN143 arc E):\n{all}"
    );
    assert!(
        all.contains("loft install probepkg"),
        "and name the cure the scope actually takes:\n{all}"
    );
    assert_eq!(
        all.matches("is the newest release").count(),
        1,
        "exactly one line — both parse passes walk this path, and a doubled notice reads \
         as two findings:\n{all}"
    );
}

/// The three silences, which are most of the design: current, offline, and nothing
/// pinned. Each is a separate reason, and any one of them failing would make the notice
/// the kind of output people learn to filter.
#[test]
fn arc_e_is_silent_when_current_offline_or_unpinned() {
    // Current: the pin IS the newest release.
    let home = empty_home("e_current");
    cache_pkg(&home, "probepkg", "0.2.0", ">=0.8");
    cached_index(&home, "probepkg", "0.2.0");
    let pkg = pinned_package(&home, "0.2.0");
    let current = run_env(&home, &pkg, "src/s.loft", &[]);
    let _ = std::fs::remove_dir_all(&home);

    // Offline: this run has opted out of the registry, so it is not asking what the
    // registry now holds.
    let home = empty_home("e_offline");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cached_index(&home, "probepkg", "0.2.0");
    let pkg = pinned_package(&home, "0.1.0");
    let offline = run_env(&home, &pkg, "src/s.loft", &[("LOFT_OFFLINE", "1")]);
    let _ = std::fs::remove_dir_all(&home);

    // Unpinned: a bare script re-decides every run, so it cannot be behind.
    let home = empty_home("e_bare");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cached_index(&home, "probepkg", "0.2.0");
    let dir = home.join("bare");
    probe_script(&dir);
    let bare = run_env(&home, &dir, "s.loft", &[]);
    let _ = std::fs::remove_dir_all(&home);

    for (label, out) in [
        ("a current pin", &current),
        ("an offline run", &offline),
        ("a bare script", &bare),
    ] {
        assert!(
            !out.contains("is the newest release"),
            "{label} has nothing to act on and must be silent:\n{out}"
        );
    }
    assert!(
        current.contains("probepkg-0.2.0") && offline.contains("probepkg-0.1.0"),
        "and each still resolved what its declaration names:\n{current}\n---\n{offline}"
    );
}

/// The off-switch, verified by its EFFECT: the same fixture that speaks above is silent
/// with it set.
#[test]
fn arc_e_the_off_switch_silences_it() {
    let home = empty_home("e_off");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cached_index(&home, "probepkg", "0.2.0");
    let pkg = pinned_package(&home, "0.1.0");
    let all = run_env(
        &home,
        &pkg,
        "src/s.loft",
        &[("LOFT_NO_UPGRADE_NOTICE", "1")],
    );
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        all.contains("probepkg-0.1.0") && !all.contains("is the newest release"),
        "LOFT_NO_UPGRADE_NOTICE must silence the notice without changing what resolves:\n{all}"
    );
}

/// The other scope that carries a declaration, and the other cure: a pinned script is
/// re-pinned with `loft pin`, not with `loft install` — which after arc D would create a
/// package around a script that never asked to be one.
#[test]
fn arc_e_a_pinned_script_is_told_to_re_pin() {
    let home = empty_home("e_sidecar");
    cache_pkg(&home, "probepkg", "0.1.0", ">=0.8");
    cached_index(&home, "probepkg", "0.2.0");
    let dir = home.join("script");
    probe_script(&dir);
    write(
        &dir.join("s.loft.lock"),
        "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\
         url = \"http://127.0.0.1:1/probepkg-0.1.0.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
         source = \"registry\"\n",
    );
    let all = run_env(&home, &dir, "s.loft", &[]);
    let _ = std::fs::remove_dir_all(&home);

    // The path is the resolved one rather than the spelling typed, so the command works
    // from wherever the reader is standing when they run it.
    assert!(
        all.contains("0.2.0 is the newest release")
            && all.contains("run: loft pin ")
            && all.contains("s.loft"),
        "a sidecar pin behind the registry must name `loft pin`:\n{all}"
    );
}

/// loft#1687 — a cached library whose OWN manifest declares a dependency, and a bare script
/// that uses it, offline.  `probedep` is extracted at 0.1.0 and 0.2.0, each answering its
/// own version; `probepkg` declares `probedep = "<range>"` and passes that answer through,
/// so the printed line names the dependency version the declared scope resolved.
fn cache_pkg_with_dep(home: &Path, range: &str) {
    for v in ["0.1.0", "0.2.0"] {
        let dir = home.join(".loft/registry").join(format!("probedep-{v}"));
        write(
            &dir.join("loft.toml"),
            &format!(
                "[package]\nname = \"probedep\"\nversion = \"{v}\"\nloft = \">=0.8\"\n\n\
                 [library]\nentry = \"src/probedep.loft\"\n"
            ),
        );
        write(
            &dir.join("src/probedep.loft"),
            &format!("pub fn dep_id() -> text {{ return \"probedep-{v}\"; }}\n"),
        );
    }
    let dir = home.join(".loft/registry/probepkg-0.1.0");
    write(
        &dir.join("loft.toml"),
        &format!(
            "[package]\nname = \"probepkg\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n\
             [library]\nentry = \"src/probepkg.loft\"\n\n\
             [dependencies]\nprobedep = \"{range}\"\n"
        ),
    );
    write(
        &dir.join("src/probepkg.loft"),
        "use probedep;\npub fn probe_id() -> text { return \"probepkg-0.1.0 via {dep_id()}\"; }\n",
    );
}

/// loft#1687 — offline, a cached library's own dependency resolves from the cache under
/// the range that library declares (PKG_REGISTRY.md § failure path 5), and never past it.
///
/// c1 `>=0.1`: both cached copies satisfy it — the newest, 0.2.0.  This is the reported
///    case (`use graphics;` offline, `mesh3d` cached, graphics asking `>=0.1.1`), which
///    answered "Library 'probedep' not found".
/// c2 `^0.1`: only 0.1.0 satisfies it — 0.1.0, although 0.2.0 is the newer copy.  The
///    bare-scope rule (newest cached) would answer 0.2.0 here, past the declaration; this
///    cell is what keeps the widening honest.
/// c3 `>=0.3`: nothing cached satisfies it — the run fails, and the message names the
///    range and the copies that are there instead of implying nothing is.
///
/// @falsified-at: hand-measured (2026-09-26) — `probe_cache_newest` taking the declared
///   scope's constraints as empty (the bare rule for every scope): c2 prints
///   `probepkg-0.1.0 via probedep-0.2.0` where this file says 0.1.0; c1 still passes.
///   CHANNEL: the value (the resolved version printed).
#[test]
fn offline_a_cached_librarys_dependency_resolves_under_its_declared_range() {
    for (tag, range, want) in [
        ("c1", ">=0.1", Some("probedep-0.2.0")),
        ("c2", "^0.1", Some("probedep-0.1.0")),
        ("c3", ">=0.3", None),
    ] {
        let home = empty_home(&format!("1687_{tag}"));
        cache_pkg_with_dep(&home, range);
        let dir = home.join("proj");
        probe_script(&dir);
        let all = run_env(&home, &dir, "s.loft", &[("LOFT_OFFLINE", "1")]);
        let _ = std::fs::remove_dir_all(&home);
        match want {
            Some(v) => assert!(
                all.contains(&format!("probepkg-0.1.0 via {v}")),
                "{tag} ({range}): the declared range must resolve {v} from the cache:\n{all}"
            ),
            None => assert!(
                all.contains("no cached copy satisfies `>=0.3`")
                    && all.contains("cached: 0.1.0, 0.2.0"),
                "{tag} ({range}): an unmet range must be named with the cached copies:\n{all}"
            ),
        }
    }
}
