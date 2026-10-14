// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#963 — a `{ path = … }` dependency resolves from the path it names.
//!
//! It did not, and the shape of the failure is what makes this worth four cells rather
//! than one: declaring the dependency was **strictly worse than saying nothing**. Without
//! the `[dependencies]` block, `--lib lib/` resolved the library and the suite passed;
//! with it, both the declaration AND `--lib` failed, and the error still read *"searched
//! lib/, lib_dirs, and sibling packages"* while `lib_dirs` held the answer.
//!
//! The cause was the dep-shadowing guard in `lib_path`. It exists for a stray same-named
//! MODULE FILE — `use server` in a package that also contains `server.loft` — and blocks
//! any candidate inside the declaring package. A path dep is inside the declaring package
//! *by definition*, so the guard matched the very directory the declaration named:
//! `probe_manifest_path_dep` resolved it, the next sweep wiped it, `--lib` resolved it
//! again, and the sweep after that wiped it too.
//!
//! ⚠ THE LAST TEST IS NOT OPTIONAL. Exempting the path dep could have been written as
//! "stop blocking in-package candidates", which passes every other cell here and silently
//! retires the rule the guard is for. `a_registry_dep_still_outranks_a_same_named_local_file`
//! is the one that fails if the exemption is widened past path deps.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// `~/.loft/registry` — where an installed package is extracted.
fn dirs_registry() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".loft/registry")
}

fn write(path: &Path, body: &str) {
    fa::create_dir_all(path.parent().unwrap()).expect("mkdir");
    fa::write(path, body).expect("write");
}

const CONSUMER_MANIFEST: &str = "[package]\nname    = \"consumer\"\nversion = \"0.1.0\"\n\
                                 loft    = \">=0.8\"\n\n[library]\nentry = \"src/consumer.loft\"\n";

/// A consumer package with an in-tree library at `lib/mylib`, plus a test that reaches it
/// through the consumer's own entry. `declare` adds the `[dependencies]` block — the only
/// thing that varies between the rows.
fn build_tree(root: &Path, declare: bool) {
    write(
        &root.join("lib/mylib/loft.toml"),
        "[package]\nname    = \"mylib\"\nversion = \"0.1.0\"\nloft    = \">=0.8\"\n\n\
         [library]\nentry = \"src/mylib.loft\"\n",
    );
    write(
        &root.join("lib/mylib/src/mylib.loft"),
        "pub fn answer() -> integer { 42 }\n",
    );
    write(
        &root.join("src/consumer.loft"),
        "pub use mylib::*;\n\npub fn doubled() -> integer { answer() * 2 }\n",
    );
    write(
        &root.join("tests/t2.loft"),
        "use consumer::*;\n\nfn test_via_the_project_library() {\n    \
         assert(doubled() == 84, \"doubled {doubled()}\");\n}\n",
    );
    let manifest = if declare {
        format!("{CONSUMER_MANIFEST}\n[dependencies]\nmylib = {{ path = \"lib/mylib\" }}\n")
    } else {
        CONSUMER_MANIFEST.to_string()
    };
    write(&root.join("loft.toml"), &manifest);
}

/// Run `loft test` on the tree, optionally with `--lib lib/`. Returns the combined output.
fn run_suite(tag: &str, declare: bool, with_lib_flag: bool) -> String {
    let root = std::env::temp_dir().join(format!("loft_963_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    build_tree(&root, declare);

    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("test");
    if with_lib_flag {
        cmd.arg("--lib").arg(root.join("lib"));
    }
    cmd.arg(root.join("tests/t2.loft"))
        .env("LOFT_TIMEOUT", "90")
        .current_dir(&root);
    let out = cmd.output().expect("spawn loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);
    all
}

/// The reported row: the declaration present, `--lib` present, and it used to FAIL.
#[test]
fn a_declared_path_dep_resolves_with_the_lib_flag() {
    let all = run_suite("declared_lib", true, true);
    assert!(
        all.contains("test result: ok"),
        "a declared path dep must not suppress the --lib search that resolves it\n{all}"
    );
}

/// The row that matters more: the declaration alone should be enough. `--lib` is a
/// developer convenience; a manifest that names where its dependency lives is the
/// package's own statement about it.
#[test]
fn a_declared_path_dep_resolves_with_no_flag_at_all() {
    let all = run_suite("declared_bare", true, false);
    assert!(
        all.contains("test result: ok"),
        "a `{{ path = … }}` dependency must resolve from the path it names\n{all}"
    );
}

/// The control that made the bug legible: WITHOUT the declaration, `--lib` already worked.
/// It has to keep working, or the fix traded one broken row for another.
#[test]
fn the_lib_flag_alone_keeps_working() {
    let all = run_suite("undeclared_lib", false, true);
    assert!(
        all.contains("test result: ok"),
        "--lib resolution must be unaffected\n{all}"
    );
}

/// And with neither, there is nothing to say where `mylib` is — that must still fail, or
/// the three greens above could be coming from some unrelated fallback.
#[test]
fn neither_a_declaration_nor_a_flag_still_fails() {
    let all = run_suite("undeclared_bare", false, false);
    assert!(
        all.contains("FAILED") || all.contains("not found"),
        "with nothing naming the library, resolution must fail\n{all}"
    );
}

/// ⚠ The rule the guard is FOR, unchanged: a package that declares a REGISTRY dependency
/// and also ships a module file of the same name must get the dependency, not the file.
///
/// This is the cell that fails if the loft#963 exemption is written as "stop blocking
/// in-package candidates" rather than "exempt the path the declaration names".
#[test]
fn a_registry_dep_still_outranks_a_same_named_local_file() {
    // Needs a registry package to outrank the local file WITH, and the only honest one
    // is a real dependency.  Self-skip rather than reach the network from a unit test:
    // a cached extraction is what makes this run offline-clean.
    let cached = fa::read_dir(dirs_registry())
        .ok()
        .into_iter()
        .flatten()
        .any(|e| e.file_name().is_some_and(|n| n.starts_with("arguments-")));
    if !cached {
        eprintln!("SKIP: no ~/.loft/registry/arguments-* extraction to outrank the local file");
        return;
    }
    let root = std::env::temp_dir().join(format!("loft_963_shadow_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    write(
        &root.join("loft.toml"),
        "[package]\nname    = \"shadowp\"\nversion = \"0.1.0\"\n\n[library]\n\
         entry = \"src/shadowp.loft\"\n\n[dependencies]\narguments = \">=0.1\"\n",
    );
    // A same-named module file whose function exists ONLY here.  If it were to win,
    // `arg_count()` would resolve and the program would print -999.
    write(
        &root.join("src/arguments.loft"),
        "pub fn arg_count() -> integer { -999 }\n",
    );
    write(
        &root.join("src/shadowp.loft"),
        "pub use arguments::*;\n\npub fn probe() -> integer { arg_count() }\n",
    );
    write(
        &root.join("run.loft"),
        "use shadowp::*;\nfn main() { print(\"{probe()}\\n\"); }\n",
    );

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(root.join("run.loft"))
        .env("LOFT_TIMEOUT", "120")
        .current_dir(&root)
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);

    assert!(
        !all.contains("-999"),
        "the local `arguments.loft` must not shadow the declared dependency\n{all}"
    );
    // ⚠ Absence of `-999` is not enough on its own: a run that failed to resolve
    // ANYTHING would satisfy it too, and this test would then pass while proving
    // nothing.  `arg_count` is defined only in the local file, so the compiler not
    // knowing it is positive evidence that the file was not bound to `arguments`.
    assert!(
        all.contains("Unknown function arg_count"),
        "expected the local module NOT to be bound — a green here without this is a \
         run that resolved nothing\n{all}"
    );
}

/// A `loft.toml` string is TOML's: a backslash starts an escape.  A Windows path written as
/// the shell prints it (`lib\mylib`) holds `\m`, which is not one, and the lexer read it as
/// `lib?ylib` and said nothing — so the run failed on what the dependency should have
/// declared ("Undefined type", "Unknown function"), naming the script and never the manifest.
/// It is reported at the manifest's line now, on every OS; the doubled spelling is what a
/// Windows author writes, and `lib_process`'s type-trigger test writes it that way.
#[test]
fn an_undoubled_backslash_in_a_path_dep_is_reported_at_the_manifest() {
    let root = std::env::temp_dir().join(format!("loft_bslash_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    build_tree(&root, false);
    let manifest =
        format!("{CONSUMER_MANIFEST}\n[dependencies]\nmylib = {{ path = \"lib\\mylib\" }}\n");
    // The dependency is the manifest's last line.
    let dep_line = format!("loft.toml:{}:", manifest.lines().count());
    write(&root.join("loft.toml"), &manifest);
    write(
        &root.join("run.loft"),
        "use consumer::*;\nfn main() { print(\"{doubled()}\\n\"); }\n",
    );
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(root.join("run.loft"))
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_ERRORS", "compact")
        .current_dir(&root)
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);
    assert!(
        !out.status.success(),
        "a manifest it could not read must fail the run\n{all}"
    );
    let at_manifest: Vec<&str> = all
        .lines()
        .filter(|l| l.contains("backslash") && l.contains(&dep_line))
        .collect();
    assert_eq!(
        at_manifest.len(),
        1,
        "the escape must be reported once, at the manifest's dependency line\n{all}"
    );
}
