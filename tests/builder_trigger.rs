// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The builder half of the auto-`use` trigger surface (`triggers.rs` § Builder triggers): a
//! library lists the free `pub fn`s a program may call BARE under `[triggers] builders`, and
//! a bare `bag(words)` then loads the package and imports that one name — the one trigger
//! that puts a name into the program's own namespace, so it is declared rather than derived.
//! The cells, each on both backends, against the fixture `tests/lib/builder_pkg`:
//!
//! - a bare call with nothing else naming the package loads it, and the built value's
//!   methods resolve;
//! - the program's own `fn bag` wins, while `builder_pkg::bag(…)` still reaches the library;
//! - a free `pub fn` the manifest does not list (`tally`) is reached only qualified;
//! - a declared name the source has only as a method (`has`) or not at all (`missing`) is
//!   dropped by the derivation and never fires.
use loft::file_access as fa;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A throwaway project that declares the fixture by path, with one script in it.
fn project(tag: &str, script: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_builder_{}_{tag}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    fa::write(
        dir.join("loft.toml"),
        format!(
            "[package]\nname = \"usesbag\"\nversion = \"0.1.0\"\n\n[dependencies]\nbuilder_pkg = {{ path = \"{}\" }}\n",
            // A TOML string: a Windows path's backslashes are doubled, as its author writes them.
            root()
                .join("tests/lib/builder_pkg")
                .display()
                .to_string()
                .replace('\\', "\\\\")
        ),
    )
    .expect("manifest");
    fa::write(dir.join("p.loft"), script).expect("script");
    dir
}

fn run(dir: &std::path::Path, backend: &str) -> (bool, String, String) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg(dir.join("p.loft"))
        .env("LOFT_TIMEOUT", "120")
        .current_dir(std::env::temp_dir())
        .output()
        .expect("spawn loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn expect(tag: &str, script: &str, want: &str) {
    let dir = project(tag, script);
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run(&dir, backend);
        assert!(
            ok && stdout == want,
            "[{tag} {backend}] expected `{want}`:\n{stdout}\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn a_bare_call_of_a_declared_builder_loads_the_library() {
    expect(
        "bare",
        "fn main() {\n  b = bag([\"pear\", \"apple\", \"pear\"]);\n  a = b.has(\"apple\");\n  p = b.has(\"plum\");\n  println(\"{a} {p} {b.size()}\");\n}\n",
        "true false 2",
    );
}

#[test]
fn the_programs_own_definition_wins_and_the_qualified_name_reaches_the_library() {
    expect(
        "own",
        "fn bag(words: vector<text>) -> integer {\n  7\n}\n\nfn main() {\n  mine = bag([\"x\"]);\n  theirs = builder_pkg::bag([\"x\", \"y\"]).size();\n  println(\"{mine} {theirs}\");\n}\n",
        "7 2",
    );
}

#[test]
fn a_free_function_that_is_not_declared_is_reached_only_qualified() {
    expect(
        "qualified",
        "fn main() {\n  println(\"{builder_pkg::tally([\"a\", \"b\", \"c\"])}\");\n}\n",
        "3",
    );
    // Bare, it is an unknown function: nothing in the scan maps `tally` to a package.
    let dir = project(
        "undeclared",
        "fn main() {\n  println(\"{tally([\"a\"])}\");\n}\n",
    );
    let (ok, stdout, stderr) = run(&dir, "--interpret");
    assert!(
        !ok && stderr.contains("tally"),
        "a bare `tally` must not auto-load the package:\n{stdout}\n{stderr}"
    );
    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn a_declared_name_the_source_does_not_carry_as_a_free_fn_never_fires() {
    // `has` is a method in the fixture and `missing` is nowhere: neither is a builder, so
    // a bare call of either is the ordinary unknown-function refusal.
    for (tag, script) in [
        ("method", "fn main() {\n  println(\"{has(\"a\")}\");\n}\n"),
        ("missing", "fn main() {\n  println(\"{missing(1)}\");\n}\n"),
    ] {
        let dir = project(tag, script);
        let (ok, stdout, stderr) = run(&dir, "--interpret");
        assert!(
            !ok && stderr.contains(tag.replace("method", "has").as_str()),
            "[{tag}] a bare call must be refused as unknown:\n{stdout}\n{stderr}"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}

/// A REGISTRY dependency fires its type and builder triggers like a `path =` one: the
/// extracted copy under the cache (`LOFT_HOME/.loft/registry/<pkg>-<version>`, the lock's pin)
/// is read in place.  Before, only the catalogue answered for a registry dependency, and
/// it carries no types: `x: sorted<Bag[key]>` was "Undefined type Bag" without a `use`.
#[test]
fn a_registry_dependency_fires_its_type_and_builder_triggers() {
    let home = std::env::temp_dir().join(format!("loft_builder_home_{}", std::process::id()));
    let _ = fa::remove_dir_all(&home);
    let pkg = home
        .join(".loft")
        .join("registry")
        .join("builder_pkg-0.1.0");
    fa::create_dir_all(pkg.join("src")).expect("mkdir");
    for f in ["loft.toml", "src/builder_pkg.loft"] {
        fa::copy(root().join("tests/lib/builder_pkg").join(f), pkg.join(f)).expect("copy");
    }
    let dir = std::env::temp_dir().join(format!("loft_builder_{}_registry", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    fa::write(
        dir.join("loft.toml"),
        "[package]\nname = \"usesbag\"\nversion = \"0.1.0\"\n\n[dependencies]\nbuilder_pkg = \">=0.1\"\n",
    )
    .expect("manifest");
    fa::write(
        dir.join("loft.lock"),
        "schema_version = 1\n\n[[package]]\nname = \"builder_pkg\"\nversion = \"0.1.0\"\nurl = \"\"\nsha256 = \"\"\nsource = \"registry\"\n",
    )
    .expect("lock");
    fa::write(
        dir.join("p.loft"),
        "fn main() {\n  b = bag([\"pear\", \"apple\", \"pear\"]);\n  x: sorted<Bag[key]> = b;\n  println(\"{x.size()} {b.has(\\\"apple\\\")}\");\n}\n",
    )
    .expect("script");
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(dir.join("p.loft"))
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_HOME", &home)
            .current_dir(std::env::temp_dir())
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.trim_end() == "2 true",
            "[{backend}] the registry dependency's type or builder trigger did not fire:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = fa::remove_dir_all(&dir);
    let _ = fa::remove_dir_all(&home);
}

/// The triggers come from the copy a `use` would load (`@FR-Disp-Trigger`).  `use` takes a `--lib` directory's
/// copy before an installed one (the testbed's "a dependency you are changing gets an explicit
/// `--lib <its worktree>`"), so the method triggers must too: a method only that copy has,
/// `shout`, fires with a declared registry dependency pinned to an older copy, and with no
/// declaration at all.  The control runs the declaring project without the flag, where the
/// pinned copy has no `shout` and the call must fail — so a pass is the override's doing.
#[test]
fn a_lib_flag_copy_fires_its_own_method_triggers() {
    let tmp = std::env::temp_dir().join(format!("loft_builder_libflag_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    let fixture = root().join("tests/lib/builder_pkg");
    // The pinned registry copy: the fixture as it is, version 0.1.0.
    let home = tmp.join("home");
    let pinned = home.join(".loft/registry/builder_pkg-0.1.0");
    // The `--lib` override: the same package one version on, with one method more.
    let override_dir = tmp.join("override");
    let newer = override_dir.join("builder_pkg");
    for (dst, version) in [(&pinned, "0.1.0"), (&newer, "0.2.0")] {
        fa::create_dir_all(dst.join("src")).expect("mkdir");
        let manifest = fa::read_to_string(fixture.join("loft.toml")).expect("manifest");
        fa::write(
            dst.join("loft.toml"),
            manifest.replace("version = \"0.1.0\"", &format!("version = \"{version}\"")),
        )
        .expect("manifest");
        let mut src = fa::read_to_string(fixture.join("src/builder_pkg.loft")).expect("source");
        if version == "0.2.0" {
            src.push_str("\npub fn shout(self: text) -> text { \"{self}!\" }\n");
        }
        fa::write(dst.join("src/builder_pkg.loft"), src).expect("source");
    }
    let script = "fn main() { println(\"hi\".shout()); }\n";
    let declaring = tmp.join("declaring");
    fa::create_dir_all(&declaring).expect("mkdir");
    fa::write(
        declaring.join("loft.toml"),
        "[package]\nname = \"usesbag\"\nversion = \"0.1.0\"\n\n[dependencies]\nbuilder_pkg = \">=0.1\"\n",
    )
    .expect("manifest");
    fa::write(
        declaring.join("loft.lock"),
        "schema_version = 1\n\n[[package]]\nname = \"builder_pkg\"\nversion = \"0.1.0\"\nurl = \"\"\nsha256 = \"\"\nsource = \"registry\"\n",
    )
    .expect("lock");
    fa::write(declaring.join("p.loft"), script).expect("script");
    let bare = tmp.join("bare");
    fa::create_dir_all(&bare).expect("mkdir");
    fa::write(
        bare.join("loft.toml"),
        "[package]\nname = \"bare\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    fa::write(bare.join("p.loft"), script).expect("script");
    let run_with = |dir: &std::path::Path, backend: &str, lib: bool| {
        let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg(backend);
        if lib {
            cmd.arg("--lib").arg(&override_dir);
        }
        let out = cmd
            .arg(dir.join("p.loft"))
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_HOME", &home)
            .env("LOFT_NO_CACHE", "1")
            .current_dir(std::env::temp_dir())
            .output()
            .expect("spawn loft");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    for backend in ["--interpret", "--native"] {
        for (dir, what) in [
            (&declaring, "a declared dependency"),
            (&bare, "no declaration"),
        ] {
            let (ok, stdout, stderr) = run_with(dir, backend, true);
            assert!(
                ok && stdout == "hi!",
                "[{backend}, {what}] the --lib copy's method did not fire:\n{stdout}\n{stderr}"
            );
        }
        let (ok, stdout, stderr) = run_with(&declaring, backend, false);
        assert!(
            !ok && stderr.contains("shout"),
            "[{backend}] without the flag the pinned copy has no `shout`, so the call must fail:\n{stdout}\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&tmp);
}

/// Write a trigger-enabled package `name` under `dir` whose one method is `method`.
fn trigger_pkg(dir: &std::path::Path, name: &str, method: &str) {
    let pkg = dir.join(name);
    fa::create_dir_all(pkg.join("src")).expect("mkdir");
    fa::write(
        pkg.join("loft.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[triggers]\nenabled = true\n"
        ),
    )
    .expect("manifest");
    fa::write(
        pkg.join(format!("src/{name}.loft")),
        format!("pub fn {method}(self: text) -> text {{ \"{name} {{self}}\" }}\n"),
    )
    .expect("source");
}

/// Run `prog` with the given `--lib` and `LOFT_LIB` directories, in an empty home so no
/// installed copy or catalogue answers: (success, stdout, stderr).
fn run_trigger(
    prog: &std::path::Path,
    backend: &str,
    lib: Option<&std::path::Path>,
    loft_lib: Option<&std::path::Path>,
    home: &std::path::Path,
) -> (bool, String, String) {
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg(backend);
    if let Some(l) = lib {
        cmd.arg("--lib").arg(l);
    }
    if let Some(l) = loft_lib {
        cmd.env("LOFT_LIB", l);
    } else {
        cmd.env_remove("LOFT_LIB");
    }
    let out = cmd
        .arg(prog)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_HOME", home)
        .env("LOFT_NO_CACHE", "1")
        .current_dir(std::env::temp_dir())
        .output()
        .expect("spawn loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// `@FR-Disp-Trigger` (loft#2095, D-disp-6): `use` searches the `LOFT_LIB` directories just
/// after `--lib`, so a bare method call reaches a trigger-enabled package found there, as it
/// reaches one under `--lib` — both when nothing declares the package and when the project's
/// manifest declares it with no other copy to load.  The control runs without `LOFT_LIB`,
/// where nothing provides the method and the call must fail.
#[test]
fn a_loft_lib_package_fires_its_method_triggers() {
    let tmp = std::env::temp_dir().join(format!("loft_trigger_loftlib_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    let home = tmp.join("home");
    fa::create_dir_all(&home).expect("mkdir");
    let solo = tmp.join("solo");
    trigger_pkg(&solo, "twc", "only_twc");
    let script = "fn main() { println(\"x\".only_twc()); }\n";
    let bare = tmp.join("bare");
    fa::create_dir_all(&bare).expect("mkdir");
    fa::write(bare.join("p.loft"), script).expect("script");
    let declaring = tmp.join("declaring");
    fa::create_dir_all(&declaring).expect("mkdir");
    fa::write(
        declaring.join("loft.toml"),
        "[package]\nname = \"usestwc\"\nversion = \"0.1.0\"\n\n[dependencies]\ntwc = \">=0.1\"\n",
    )
    .expect("manifest");
    fa::write(declaring.join("p.loft"), script).expect("script");
    for backend in ["--interpret", "--native"] {
        for (dir, what) in [
            (&bare, "no declaration"),
            (&declaring, "a declared dependency"),
        ] {
            let (ok, stdout, stderr) =
                run_trigger(&dir.join("p.loft"), backend, None, Some(&solo), &home);
            assert!(
                ok && stdout == "twc x",
                "[{backend}, {what}] the LOFT_LIB package's method did not fire:\n{stdout}\n{stderr}"
            );
        }
        let (ok, stdout, stderr) = run_trigger(&bare.join("p.loft"), backend, None, None, &home);
        assert!(
            !ok && stderr.contains("only_twc"),
            "[{backend}] without LOFT_LIB nothing provides `only_twc`, so the call must fail:\n{stdout}\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&tmp);
}

/// `@FR-Disp-Trigger` (loft#2094, D-disp-5): a method two trigger-enabled packages of one
/// search step both provide is refused by name — both packages and the `use` that settles
/// it — instead of reading as an unknown method, and no directory order picks one.  The
/// control adds the `use`, which must run.
#[test]
fn a_method_two_packages_provide_is_refused_naming_both() {
    let tmp = std::env::temp_dir().join(format!("loft_trigger_twice_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    let home = tmp.join("home");
    fa::create_dir_all(&home).expect("mkdir");
    let libs = tmp.join("libs");
    trigger_pkg(&libs, "twa", "hum");
    trigger_pkg(&libs, "twb", "hum");
    let prog = tmp.join("both.loft");
    fa::write(&prog, "fn main() { println(\"x\".hum()); }\n").expect("script");
    let settled = tmp.join("settled.loft");
    fa::write(
        &settled,
        "use twb::*;\nfn main() { println(\"x\".hum()); }\n",
    )
    .expect("script");
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run_trigger(&prog, backend, Some(&libs), None, &home);
        assert!(
            !ok && stderr.contains("`hum` is provided by both `twa` and `twb`")
                && stderr.contains("`use twa::*;` or `use twb::*;`"),
            "[{backend}] the ambiguous trigger was not refused by name:\n{stdout}\n{stderr}"
        );
        let (ok, stdout, stderr) = run_trigger(&settled, backend, Some(&libs), None, &home);
        assert!(
            ok && stdout == "twb x",
            "[{backend}] `use twb::*;` must settle it:\n{stdout}\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&tmp);
}

/// `@FR-Disp-Trigger` (loft#2094, D-disp-5), the catalogue half: a method two PUBLISHED
/// packages provide is left out of the catalogue's trigger map (no package is guessed), and
/// the call is refused naming both instead of reading as an unknown method.  The index is a
/// fake one in an otherwise empty home, so nothing installed answers.
#[test]
fn a_method_two_catalogue_packages_provide_is_refused_naming_both() {
    let tmp = std::env::temp_dir().join(format!("loft_trigger_catalog_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    let home = tmp.join("home");
    fa::create_dir_all(home.join(".loft/registry")).expect("mkdir");
    let version = |trigger: &str| {
        format!(
            r#"{{ "categories": [], "yanked": [], "versions": {{ "0.1.0": {{
                "url": "https://example.invalid/p.tar.gz", "sha256": "s", "size": 1,
                "loft": ">=0.8", "triggers": ["{trigger}"],
                "published": "2026-10-20T00:00:00Z" }} }} }}"#
        )
    };
    fa::write(
        home.join(".loft/registry/index.json"),
        format!(
            r#"{{ "schema_version": 1, "updated": "2026-10-20T00:00:00Z", "packages": {{
                "cata": {}, "catb": {} }} }}"#,
            version("hum:text"),
            version("hum:integer")
        ),
    )
    .expect("index");
    let prog = tmp.join("p.loft");
    fa::write(&prog, "fn main() { println(\"x\".hum()); }\n").expect("script");
    for backend in ["--interpret", "--native"] {
        let (ok, stdout, stderr) = run_trigger(&prog, backend, None, None, &home);
        assert!(
            !ok && stderr.contains("`hum` is provided by both `cata` and `catb`"),
            "[{backend}] the catalogue's ambiguous trigger was not refused by name:\n{stdout}\n{stderr}"
        );
    }
    let _ = fa::remove_dir_all(&tmp);
}
