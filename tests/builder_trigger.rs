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
