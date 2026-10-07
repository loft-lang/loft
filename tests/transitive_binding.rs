// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1849 — a `use` inside a dependency is bound by the ROOT project's declaration of the
//! package, and by the range the importing package names for it.
//!
//! PACKAGES.md: "The root project's declared constraints pin the whole tree, including
//! packages pulled in transitively by a `use` inside a dependency", and a package loads
//! once.  The defect: a root that declared `probepkg` as a PATH dependency did not reach a
//! `use probepkg` inside `mid`, which resolved from the registry cache instead — and that
//! copy, loaded first, became the program's.  Which copy a program ran against depended on
//! the order of its `use` lines, and the importer's own range was not checked against it.
//!
//! Hermetic: `LOFT_HOME` is a per-test cache holding `probepkg` 0.1.0 and 0.2.0, and the
//! registry URL cannot be fetched, so the cache answers whatever the registry would.  The
//! root's path copy is 0.3.0.  Every copy's `probe_id()` names itself, so a cell cannot
//! pass by loading the wrong one.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().unwrap()).expect("mkdir");
    fa::write(path, body).expect("write");
}

/// A package directory: its manifest (with `deps` as `[dependencies]` lines) and an entry
/// whose `probe_id()` — or, for a middle package, `<name>_id()` — names what it loaded.
fn package(dir: &Path, name: &str, version: &str, deps: &str, body: &str) {
    write(
        &dir.join("loft.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\n[library]\n\
             entry = \"src/{name}.loft\"\n\n[dependencies]\n{deps}\n"
        ),
    );
    write(&dir.join("src").join(format!("{name}.loft")), body);
}

/// A cached registry copy of `probepkg`, exactly where `install_one` leaves one.
fn cached_probe(home: &Path, version: &str) {
    package(
        &home
            .join(".loft/registry")
            .join(format!("probepkg-{version}")),
        "probepkg",
        version,
        "",
        &format!("pub fn probe_id() -> text {{ \"probepkg-{version}\" }}\n"),
    );
}

/// A middle package that names `probepkg` with `range` and reports what it loaded.
fn middle(dir: &Path, name: &str, range: &str, extra_deps: &str, uses: &str) {
    package(
        dir,
        name,
        "0.1.0",
        &format!("probepkg = \"{range}\"\n{extra_deps}"),
        &format!("{uses}\npub fn {name}_id() -> text {{ probe_id() }}\n"),
    );
}

struct Fixture {
    home: PathBuf,
    root: PathBuf,
}

/// A fresh home (0.1.0 and 0.2.0 cached) and an empty root project directory.
fn fixture(tag: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!("loft_1849_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&base);
    let home = base.join("home");
    cached_probe(&home, "0.1.0");
    cached_probe(&home, "0.2.0");
    Fixture {
        home,
        root: base.join("root"),
    }
}

impl Fixture {
    /// The root project: `deps` are its `[dependencies]`, `script` its `src/main.loft`.
    fn root(&self, deps: &str, script: &str) {
        write(
            &self.root.join("loft.toml"),
            &format!("[package]\nname = \"root\"\nversion = \"0.1.0\"\n\n[dependencies]\n{deps}\n"),
        );
        write(&self.root.join("src/main.loft"), script);
    }

    /// The root's path copy of `probepkg`, version 0.3.0.
    fn path_copy(&self) {
        package(
            &self.root.join("probe_local"),
            "probepkg",
            "0.3.0",
            "",
            "pub fn probe_id() -> text { \"probepkg-path-0.3.0\" }\n",
        );
    }

    fn run(&self, backend: &str) -> String {
        let out = loft::platform::process::harness_command(loft_bin())
            .args([backend, "src/main.loft"])
            .env("LOFT_HOME", &self.home)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("LOFT_REGISTRY_URL", "http://127.0.0.1:1/index.json")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "300")
            .current_dir(&self.root)
            .output()
            .expect("spawn loft");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    /// The root's `loft.lock` must not record a registry `probepkg` when the root declared a
    /// path copy: a run that wrote one would pin the next run to the copy this one refused.
    fn lock_has_no_registry_probe(&self) -> bool {
        fa::read_to_string(self.root.join("loft.lock"))
            .map_or(true, |l| !l.contains("name = \"probepkg\""))
    }
}

const ROOT_PATH: &str = "probepkg = { path = \"probe_local\" }\nmid = { path = \"mid\" }";

/// The root's path copy answers the middle package's `use`, in either order of the root's
/// own `use` lines, on both backends — where the cache's newest (0.2.0) used to win when
/// `use mid` came first.
#[test]
fn the_roots_path_copy_answers_a_use_inside_a_dependency() {
    for (tag, order) in [
        ("dep_first", "use mid::*;\nuse probepkg::*;"),
        ("root_first", "use probepkg::*;\nuse mid::*;"),
    ] {
        let fx = fixture(tag);
        fx.path_copy();
        middle(
            &fx.root.join("mid"),
            "mid",
            ">=0.1.0",
            "",
            "use probepkg::*;",
        );
        fx.root(
            ROOT_PATH,
            &format!("{order}\nfn main() {{ println(\"{{mid_id()}} {{probe_id()}}\"); }}\n"),
        );
        for backend in ["--interpret", "--native"] {
            let out = fx.run(backend);
            assert!(
                out.contains("probepkg-path-0.3.0 probepkg-path-0.3.0"),
                "[{tag} {backend}] both the dependency and the root get the root's path copy\n{out}"
            );
        }
        assert!(
            fx.lock_has_no_registry_probe(),
            "[{tag}] no registry copy pinned"
        );
    }
}

/// The middle package's own range still binds the copy the root names: 0.3.0 against
/// `>=9.0.0` is refused, naming both declarations, whichever `use` loaded the copy first.
#[test]
fn a_root_copy_the_dependency_excludes_is_refused_in_either_order() {
    for (tag, order) in [
        ("unmet_dep_first", "use mid::*;\nuse probepkg::*;"),
        ("unmet_root_first", "use probepkg::*;\nuse mid::*;"),
    ] {
        let fx = fixture(tag);
        fx.path_copy();
        middle(
            &fx.root.join("mid"),
            "mid",
            ">=9.0.0",
            "",
            "use probepkg::*;",
        );
        fx.root(
            ROOT_PATH,
            &format!("{order}\nfn main() {{ println(\"{{mid_id()}}\"); }}\n"),
        );
        let out = fx.run("--interpret");
        assert!(
            out.contains("`mid` needs `probepkg >=9.0.0`")
                && out.contains("is 0.3.0 — the two declarations disagree"),
            "[{tag}] refused, naming the range and the copy\n{out}"
        );
        assert!(!out.contains("probepkg-"), "[{tag}] and nothing ran\n{out}");
    }
}

/// With no root declaration the dependency's range picks from the cache, and an
/// unsatisfiable one is refused rather than answered with whatever is cached — the
/// controls that say the root-path half changed nothing else.
#[test]
fn without_a_root_declaration_the_dependencys_range_decides() {
    let fx = fixture("range_met");
    middle(
        &fx.root.join("mid"),
        "mid",
        ">=0.2.0",
        "",
        "use probepkg::*;",
    );
    fx.root(
        "mid = { path = \"mid\" }",
        "use mid::*;\nfn main() { println(mid_id()); }\n",
    );
    let out = fx.run("--interpret");
    assert!(
        out.contains("probepkg-0.2.0"),
        "the newest cached copy the range admits\n{out}"
    );

    let fx = fixture("range_unmet");
    middle(
        &fx.root.join("mid"),
        "mid",
        ">=9.0.0",
        "",
        "use probepkg::*;",
    );
    fx.root(
        "mid = { path = \"mid\" }",
        "use mid::*;\nfn main() { println(mid_id()); }\n",
    );
    let out = fx.run("--interpret");
    assert!(
        out.contains("no cached copy satisfies `>=9.0.0`") && !out.contains("probepkg-0."),
        "refused, not answered with a cached copy outside the range\n{out}"
    );
}

/// Depth two, and two dependencies sharing one transitive package: every `use probepkg`
/// in the program, however deep and from whichever package, gets the root's one copy.
#[test]
fn the_roots_copy_answers_every_depth_and_every_sharer() {
    let fx = fixture("deep_shared");
    fx.path_copy();
    // root -> mid -> inner -> probepkg, and root -> mid2 -> probepkg.
    middle(
        &fx.root.join("inner"),
        "inner",
        ">=0.1.0",
        "",
        "use probepkg::*;",
    );
    package(
        &fx.root.join("mid"),
        "mid",
        "0.1.0",
        "inner = { path = \"../inner\" }",
        "use inner::*;\npub fn mid_id() -> text { inner_id() }\n",
    );
    middle(
        &fx.root.join("mid2"),
        "mid2",
        ">=0.1.0",
        "",
        "use probepkg::*;",
    );
    fx.root(
        &format!("{ROOT_PATH}\nmid2 = {{ path = \"mid2\" }}"),
        "use mid::*;\nuse mid2::*;\nfn main() { println(\"{mid_id()} {mid2_id()}\"); }\n",
    );
    let out = fx.run("--interpret");
    assert!(
        out.contains("probepkg-path-0.3.0 probepkg-path-0.3.0"),
        "depth two and a second sharer both get the root's copy\n{out}"
    );
    assert!(fx.lock_has_no_registry_probe(), "no registry copy pinned");
}
