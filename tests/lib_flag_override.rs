// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@C173` — which copy of a library a `use` loads: what this run says, then what the project
//! declares, then what merely lies in a directory.
//!
//! - A `--lib` directory of the command line outranks everything but the importing package's
//!   own name and a module beside the importing file, and says so when it replaces a pinned or
//!   declared library (`advice[lib-flag-override]`).
//! - A `{ path = … }` dependency and the lock's pin outrank a `lib/` or a same-named file in
//!   the working directory or beside the script.
//! - A name nothing pins or declares resolves as before.
//!
//! Each cell's value names the copy that ran, so a cell cannot pass by loading the wrong one.
//! Hermetic: every cell builds its own tree and its own `~/.loft` (a registry cache holding
//! one extracted package) under the temp dir, the registry URL is unreachable, and
//! `LOFT_NO_CACHE` keeps the whole-program cache from answering for a previous cell.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fa::create_dir_all(parent).expect("mkdir");
    }
    fa::write(path, body).expect("write");
}

/// A fresh `~/.loft` for the cell `tag`, its registry cache holding `probepkg 0.1.0` extracted
/// as `install_one` leaves it; `probe_id()` answers `probepkg-0.1.0`.
fn home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("loft_c173_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&home);
    let pkg = home.join(".loft/registry/probepkg-0.1.0");
    write(
        &pkg.join("loft.toml"),
        "[package]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/probepkg.loft\"\n",
    );
    write(
        &pkg.join("src/probepkg.loft"),
        "pub fn probe_id() -> text { \"probepkg-0.1.0\" }\n",
    );
    home
}

/// A `probepkg.loft` answering `what`.
fn copy_answering(what: &str) -> String {
    format!("pub fn probe_id() -> text {{ \"{what}\" }}\n")
}

/// The lock that pins `probepkg 0.1.0` (an unreachable url: a cell must not fetch).
const PIN: &str = "schema_version = 1\n\n[[package]]\nname = \"probepkg\"\nversion = \"0.1.0\"\n\
    url = \"http://127.0.0.1:1/probepkg-0.1.0.tar.gz\"\n\
    sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\
    source = \"registry\"\n";

const SCRIPT: &str = "use probepkg::*;\nfn main() { println(probe_id()); }\n";

/// A package `proj` that declares and pins `probepkg`, under `home`.
fn pinned_project(home: &Path) -> PathBuf {
    let proj = home.join("proj");
    write(
        &proj.join("loft.toml"),
        "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n\n[dependencies]\nprobepkg = \"0.1.0\"\n",
    );
    write(&proj.join("loft.lock"), PIN);
    write(&proj.join("s.loft"), SCRIPT);
    proj
}

/// Run `args` from `cwd` against `home`: (stdout, stderr).
fn run(home: &Path, cwd: &Path, args: &[&str]) -> (String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .args(args)
        .env("LOFT_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOFT_REGISTRY_URL", "http://127.0.0.1:1/index.json")
        .env("LOFT_OFFLINE", "1")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120")
        .current_dir(cwd)
        .output()
        .expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A pinned name loads its pin, whatever lies in the working directory or beside the script:
/// a `lib/probepkg.loft` or a `probepkg.loft` in the directory the program is run from, and a
/// `lib/probepkg.loft` in the package itself.  Before `@C173` each of them answered.
#[test]
fn a_pinned_name_loads_its_pin_whatever_lies_in_a_directory() {
    let home = home("pinned");
    let proj = pinned_project(&home);
    let elsewhere = home.join("elsewhere");
    write(
        &elsewhere.join("lib/probepkg.loft"),
        &copy_answering("cwd lib"),
    );
    write(
        &elsewhere.join("probepkg.loft"),
        &copy_answering("cwd file"),
    );
    write(
        &proj.join("lib/probepkg.loft"),
        &copy_answering("package lib"),
    );
    let script = proj.join("s.loft").to_string_lossy().into_owned();
    let (out, err) = run(&home, &elsewhere, &["--interpret", &script]);
    assert_eq!(out, "probepkg-0.1.0", "run from another directory\n{err}");
    let (out, err) = run(&home, &proj, &["--interpret", "s.loft"]);
    assert_eq!(out, "probepkg-0.1.0", "run from the package\n{err}");
    // A bare script pinned by `loft pin` (its sidecar lock) holds the same way.
    let bare = home.join("bare");
    write(&bare.join("s.loft"), SCRIPT);
    write(&bare.join("s.loft.lock"), PIN);
    let script = bare.join("s.loft").to_string_lossy().into_owned();
    let (out, err) = run(&home, &elsewhere, &["--interpret", &script]);
    assert_eq!(out, "probepkg-0.1.0", "a sidecar pin\n{err}");
}

/// `--lib` outranks the pin — a library can always be debugged and fixed — and says which
/// copy runs, once.
#[test]
fn a_lib_flag_outranks_a_pin_and_says_so() {
    let home = home("flag_pin");
    let proj = pinned_project(&home);
    let over = home.join("over");
    write(&over.join("probepkg.loft"), &copy_answering("my fix"));
    let over_s = over.to_string_lossy().into_owned();
    let (out, err) = run(&home, &proj, &["--interpret", "--lib", &over_s, "s.loft"]);
    assert_eq!(out, "my fix", "the --lib copy runs\n{err}");
    assert!(
        err.contains("advice[lib-flag-override]") && err.contains("the pinned probepkg 0.1.0"),
        "and it is said which copy runs instead of the pin\n{err}"
    );
    assert_eq!(err.matches("lib-flag-override").count(), 1, "once\n{err}");
}

/// `--lib` outranks a `lib/` in the working directory too.  The copy behind the flag cannot
/// parse, so its parse error is the proof that it was the one read; nothing is said, since
/// nothing the project pins or declares was replaced.
#[test]
fn a_lib_flag_outranks_the_working_directorys_lib() {
    let home = home("flag_cwd");
    let proj = home.join("plain");
    write(
        &proj.join("lib/who.loft"),
        "pub fn who() -> text { \"project lib\" }\n",
    );
    write(
        &proj.join("main.loft"),
        "use who::*;\nfn main() { println(\"{who()}\"); }\n",
    );
    let over = home.join("over");
    write(
        &over.join("who.loft"),
        "pub fn who() -> text { \"override\" }\nthis line is not loft\n",
    );
    let over_s = over.to_string_lossy().into_owned();
    let (out, err) = run(
        &home,
        &proj,
        &["--interpret", "--lib", &over_s, "main.loft"],
    );
    assert!(
        !out.contains("project lib") && err.contains("this line is not loft"),
        "the copy behind the flag is what parsed\nstdout: {out}\n{err}"
    );
    assert!(
        !err.contains("lib-flag-override"),
        "nothing pinned was replaced\n{err}"
    );
}

/// A `{ path = … }` dependency outranks a same-named `lib/` in the working directory.
#[test]
fn a_declared_path_dependency_outranks_a_stray_lib() {
    let home = home("path_dep");
    let proj = home.join("proj");
    write(
        &proj.join("loft.toml"),
        "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n\n[dependencies]\nprobepkg = { path = \"dev/probepkg\" }\n",
    );
    write(
        &proj.join("dev/probepkg/loft.toml"),
        "[package]\nname = \"probepkg\"\nversion = \"0.1.0\"\n",
    );
    write(
        &proj.join("dev/probepkg/src/probepkg.loft"),
        &copy_answering("declared copy"),
    );
    write(&proj.join("s.loft"), SCRIPT);
    let elsewhere = home.join("elsewhere");
    write(
        &elsewhere.join("lib/probepkg.loft"),
        &copy_answering("cwd lib"),
    );
    let script = proj.join("s.loft").to_string_lossy().into_owned();
    let (out, err) = run(&home, &elsewhere, &["--interpret", &script]);
    assert_eq!(out, "declared copy", "{err}");
}

/// A module beside the importing file is not rebound by `--lib`: it is the file's own source.
#[test]
fn a_module_beside_the_script_is_not_rebound_by_a_flag() {
    let home = home("beside");
    let dir = home.join("app");
    write(
        &dir.join("helper.loft"),
        "pub fn who() -> text { \"beside\" }\n",
    );
    write(
        &dir.join("main.loft"),
        "use helper;\nfn main() { println(helper::who()); }\n",
    );
    let over = home.join("over");
    write(
        &over.join("helper.loft"),
        "pub fn who() -> text { \"flag\" }\n",
    );
    let over_s = over.to_string_lossy().into_owned();
    let (out, err) = run(&home, &dir, &["--interpret", "--lib", &over_s, "main.loft"]);
    assert_eq!(out, "beside", "{err}");
}
