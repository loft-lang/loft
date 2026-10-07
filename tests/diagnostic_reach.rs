// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1260 — **a diagnostic reaches only whoever can act on its cure.**
//!
//! The reach axis is orthogonal to the tier one: `Level` decides whether ignoring a
//! diagnostic can produce a wrong result, and so whether it gates CI; reach decides who is
//! addressed at all.  Every `warning` and `advice` loft emits names a cure that is an edit
//! at the site it points at, so one pointing into a dependency is noise that reads as the
//! reader's own defect — the Parser chapter of the reference, four `parser::parse` calls
//! long, printed eleven notes about the internals of two libraries the reader did not write.
//!
//! What makes this delicate is not the silencing, it is **the scope**.  The obvious rule —
//! "the entry file is mine" — is wrong, and was already shipped wrong: under `loft test` a
//! package's entry is `tests/*.loft` while the code under review is `src/*.loft`, so an
//! entry-file rule silences a library's lints in the one run that exists to catch them.
//! `same_lint_reaches_the_library_author_who_can_fix_it` is the cell that pins that, and it
//! is the one that failed before this landed: `linked-group-apart` gated itself on
//! `Data::source_is_owned` (`source == MAIN_SOURCE`) and was silent in its own package's
//! test run while firing for the same struct in an owned program.
//!
//! The negative cells are worth as little as their positive twins are worth: a build that
//! prints nothing at all passes every "is quiet" assertion.  So each quiet cell is paired
//! with a loud one over the SAME source, differing only in who is compiling it.
//!
//! Measured before the fix, on `7b7a8774`: `tests/docs/16-parser.loft` printed 12
//! diagnostics, all of them about `lib/parser.loft` and `lib/code.loft` and none about the
//! chapter — so `a_dependencys_lints_do_not_reach_its_consumer` failed there. And
//! `linked-group-apart` fired for a struct in an owned program while staying silent for the
//! same struct in its own package's test run, which is
//! `same_lint_reaches_the_library_author_who_can_fix_it` failing in the other direction.
//! Both are 0 and loud respectively now.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A package whose `src/` carries one `advice` (`omitted-field-zero`) and one
/// `linked-group-apart`, plus a test that exercises it.
fn write_package(root: &Path) {
    let src = root.join("mylib/src");
    let tests = root.join("mylib/tests");
    fa::create_dir_all(&src).expect("src");
    fa::create_dir_all(&tests).expect("tests");
    fa::write(
        root.join("mylib/loft.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\ncategories = [\"testing\"]\n",
    )
    .expect("manifest");
    fa::write(
        src.join("mylib.loft"),
        // `Thing { a: 1 }` omits two fields; `World` declares two collections over `E`
        // with an unrelated field between them.
        "pub struct E {\n  id: integer,\n  name: text\n}\n\
         pub struct World {\n  entities: vector<E>,\n  tick: integer,\n  \
         spawn_index: hash<E[id]>\n}\n\
         pub struct Thing {\n  pub a: integer,\n  b: integer,\n  c: integer\n}\n\
         pub fn make() -> Thing {\n  Thing { a: 1 }\n}\n",
    )
    .expect("lib source");
    fa::write(
        tests.join("t.loft"),
        "use mylib;\nfn test_make() {\n  t = mylib::make();\n  assert(t.a == 1, \"a\");\n}\n",
    )
    .expect("lib test");
}

/// Every diagnostic line in a run's combined output, whatever the renderer's casing.
fn diagnostics(out: &std::process::Output) -> Vec<String> {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("advice")
                || t.starts_with("Advice")
                || t.starts_with("warning")
                || t.starts_with("Warning")
        })
        .map(str::to_string)
        .collect()
}

/// A library author running the package's own tests sees the lints in its `src/`.
///
/// This is the cell the entry-file rule fails: the entry here is `tests/t.loft`, and the
/// code both lints point at is `src/mylib.loft`.
#[test]
fn same_lint_reaches_the_library_author_who_can_fix_it() {
    let tmp = tempdir("reach-author");
    write_package(&tmp);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("test")
        .current_dir(tmp.join("mylib"))
        .output()
        .expect("loft test");
    let got = diagnostics(&out);
    let joined = got.join("\n");
    assert!(
        joined.contains("omitted-field-zero"),
        "the package's own test run must show its own advice, got:\n{joined}"
    );
    assert!(
        joined.contains("linked-group-apart"),
        "the package's own test run must show a lint about its own struct — an entry-file \
         scope silences exactly this, because the entry is tests/ and the struct is src/. \
         Got:\n{joined}"
    );
    fa::remove_dir_all(&tmp).ok();
}

/// A consumer of that package sees none of it: they cannot edit `src/mylib.loft`.
///
/// Paired with the cell above over the SAME source, so "quiet" cannot be a build that
/// prints nothing.
#[test]
fn a_dependencys_lints_do_not_reach_its_consumer() {
    let tmp = tempdir("reach-consumer");
    write_package(&tmp);
    let app = tmp.join("app");
    fa::create_dir_all(&app).expect("app dir");
    fa::write(
        app.join("app.loft"),
        "use mylib;\nfn main() {\n  t = mylib::make();\n  println(\"{t.a}\");\n}\n",
    )
    .expect("app source");
    let out = loft::platform::process::harness_command(loft_bin())
        .args([
            "--interpret",
            "app.loft",
            "--lib",
            tmp.join("mylib/src").to_string_lossy().as_ref(),
        ])
        .current_dir(&app)
        .output()
        .expect("loft run");
    let got = diagnostics(&out);
    assert!(
        got.is_empty(),
        "a consumer cannot edit the dependency these point at:\n{}",
        got.join("\n")
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains('1'),
        "the program must still have RUN — a silenced diagnostic is not a silenced program"
    );
    fa::remove_dir_all(&tmp).ok();
}

/// The consumer's OWN code still lints. The gate is about whose file the caret lands in,
/// not about being a consumer.
#[test]
fn a_consumers_own_code_still_lints() {
    let tmp = tempdir("reach-own");
    write_package(&tmp);
    let app = tmp.join("app");
    fa::create_dir_all(&app).expect("app dir");
    fa::write(
        app.join("app.loft"),
        "struct Own {\n  a: integer,\n  b: integer\n}\n\
         fn main() {\n  o = Own { a: 1 };\n  println(\"{o.a}\");\n}\n",
    )
    .expect("app source");
    let out = loft::platform::process::harness_command(loft_bin())
        .args(["--interpret", "app.loft"])
        .current_dir(&app)
        .output()
        .expect("loft run");
    let joined = diagnostics(&out).join("\n");
    assert!(
        joined.contains("omitted-field-zero"),
        "the author's own partial literal must still be advised, got:\n{joined}"
    );
    fa::remove_dir_all(&tmp).ok();
}

/// A bare multi-file program: the module BESIDE the entry is the author's own.
///
/// The scope for a program with no `loft.toml` is the entry's DIRECTORY, not the entry
/// file. `main.loft` plus the modules next to it is an ordinary shape, and scoping to the
/// one file would drop the lints of every module but that one — the same class of mistake
/// as scoping a package to its entry.
#[test]
fn a_sibling_module_of_a_bare_script_still_lints() {
    let tmp = tempdir("reach-sibling");
    fa::write(
        tmp.join("helper1260.loft"),
        "pub struct Side {\n  a: integer,\n  b: integer\n}\n         pub fn side() -> Side {\n  Side { a: 1 }\n}\n",
    )
    .expect("sibling module");
    fa::write(
        tmp.join("main.loft"),
        "use helper1260;\nfn main() {\n  println(\"{helper1260::side().a}\");\n}\n",
    )
    .expect("entry");
    let out = loft::platform::process::harness_command(loft_bin())
        .args(["--interpret", "main.loft"])
        .current_dir(&tmp)
        .output()
        .expect("loft run");
    let joined = diagnostics(&out).join("\n");
    assert!(
        joined.contains("omitted-field-zero"),
        "a module beside the entry is the author's own; got:\n{joined}"
    );
    fa::remove_dir_all(&tmp).ok();
}

/// An ERROR in a dependency still reaches the consumer: a program that will not run has to
/// say so whoever is reading. Only lints are addressed.
#[test]
fn an_error_is_never_dropped_by_reach() {
    let tmp = tempdir("reach-error");
    write_package(&tmp);
    let app = tmp.join("app");
    fa::create_dir_all(&app).expect("app dir");
    fa::write(
        app.join("app.loft"),
        "use mylib;\nfn main() {\n  println(\"{mylib::make(1, 2, 3).a}\");\n}\n",
    )
    .expect("app source");
    let out = loft::platform::process::harness_command(loft_bin())
        .args([
            "--interpret",
            "app.loft",
            "--lib",
            tmp.join("mylib/src").to_string_lossy().as_ref(),
        ])
        .current_dir(&app)
        .output()
        .expect("loft run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.to_lowercase().contains("error"),
        "a call the library cannot satisfy must still be refused, got:\n{text}"
    );
    fa::remove_dir_all(&tmp).ok();
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "loft-1260-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fa::create_dir_all(&dir).expect("tempdir");
    dir
}
