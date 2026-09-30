// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T2-11: external library package layout.
//! Verifies that `use mylib;` resolves `<lib-dir>/<id>/src/<id>.loft`
//! when a `loft.toml` manifest is present.

extern crate loft;

use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::platform::sep_str;
use loft::scopes;

/// Confirm that lib_path() locates a library stored in the packaged directory
/// layout: `tests/lib/testpkg/src/testpkg.loft` via `lib_dirs`.
#[test]
fn package_layout_use_finds_src_subdir() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}package_test_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no parse errors; diagnostics: {:?}",
        p.diagnostics.lines()
    );
}

/// Confirm that a version requirement in `loft.toml` that exceeds the
/// current interpreter version produces a fatal diagnostic.
#[test]
fn package_layout_version_mismatch_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    // testpkg_future requires loft >= 99.0, which should always fail.
    p.parse(
        &format!("tests{s}lib{s}package_version_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a version-mismatch error"
    );
}

/// The floor is asked on EVERY path that adopts a manifest, so the `use` ORDER cannot
/// decide it: here `testpkg_future` (floor 99999.0) is reached first as a SIBLING through
/// `testpkg_uses_future`'s own `use`, and the direct `use` after it deduplicates.  Before
/// @PLN174 F6 the sibling path adopted the manifest without the check, and this program
/// compiled (`use testpkg_future; use testpkg_uses_future;` — the other order — did not).
#[test]
fn package_floor_holds_when_the_package_is_adopted_as_a_sibling_first() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_version_order_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "the sibling-adopted package's floor must refuse: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("requires loft")),
        "the refusal must name the floor: {:?}",
        p.diagnostics.lines()
    );
}

/// A manifest REFUSAL speaks alone: the package was found and rejected, so the search
/// that follows must not also report it missing.
///
/// The three manifest gates above (version, malformed constraint, contract) each raise a
/// fatal and then resolve to nothing, and "nothing" is what the `use` arm reads as "not
/// found".  The author of a package that needs a newer loft was therefore told both that
/// it needs a newer loft AND that it does not exist — and the second sentence is the one
/// that reads like the answer, because it names the search that just ran.
///
/// Checked on the version gate and paired with its control below, since a suppression is
/// the kind of fix that passes by silencing more than it was asked to.
#[test]
fn a_refused_package_is_not_also_reported_missing() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_version_test_main.loft"),
        false,
    );
    let said = p.diagnostics.lines().join("\n");
    assert!(
        said.contains("requires loft"),
        "the version refusal must still be reported; got: {said}"
    );
    assert!(
        !said.contains("not found"),
        "a package that was FOUND and refused must not also be reported missing; got: {said}"
    );
}

/// The control for the suppression above — a library that genuinely is not there still
/// says so.  Without this cell, deleting the "not found" arm entirely would pass.
#[test]
fn a_library_that_is_really_absent_still_says_not_found() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_absent_test_main.loft"),
        false,
    );
    let said = p.diagnostics.lines().join("\n");
    assert!(
        said.contains("not found"),
        "an absent library must still be reported missing; got: {said}"
    );
}

/// @PLN102 arc B — a package whose `loft.toml` carries a MALFORMED version
/// constraint (`^0.9`, an unsupported operator) is rejected LOUDLY at load,
/// not silently accepted.  Before arc B, `check_version` degraded any non-`>=`
/// form to `0.0.0` and always passed; the caret would have loaded fine.
#[test]
fn arc_b_malformed_constraint_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_badconstraint_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a fatal for the malformed version constraint '^0.9'"
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("invalid loft version requirement")),
        "Expected the malformed-constraint diagnostic, got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN102 arc B — a package with an UPPER bound the running interpreter does
/// not meet (`<=0.1` against calendar-versioned loft) is rejected.  This is the
/// core regression: before arc B the upper bound was silently ignored and the
/// package loaded, the category-S silent failure the plan removes.
#[test]
fn arc_b_unsatisfiable_upper_bound_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_upperbound_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a fatal for the unsatisfiable upper bound '<=0.1'; \
         before arc B this loaded silently"
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("requires loft")),
        "Expected the version-requirement diagnostic, got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN102 arc B-semantic — a package that requires a compatibility `contract`
/// newer than this loft provides (`CONTRACT_VERSION` is 0 pre-1.0) is a hard,
/// loud reject: loft is too old for the library's epoch.
#[test]
fn arc_b_contract_too_new_is_fatal() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_contract_future_test_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected a fatal for a contract requirement newer than this loft"
    );
    assert!(
        p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("requires loft contract")),
        "Expected the contract-too-old diagnostic, got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN102 arc B-semantic — a package declaring the CURRENT contract epoch loads
/// clean.  Guards against the gate becoming a blanket reject (a vacuous
/// too-new test would pass even if every contract were rejected).
#[test]
fn arc_b_contract_current_loads_clean() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(
        &format!("tests{s}lib{s}package_contract_ok_test_main.loft"),
        false,
    );
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "A package at the current contract epoch should load; diagnostics: {:?}",
        p.diagnostics.lines()
    );
}

/// P129: native_packages must not contain duplicate crate entries.
/// A package with `[native] crate` parsed through lib_path_manifest should
/// not produce a second entry if register_native_manifest already added it.
#[test]
fn p129_no_duplicate_native_packages() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    // Parse a file that uses the native_crate_pkg package.
    p.parse(
        &format!("tests{s}lib{s}native_crate_import_main.loft"),
        false,
    );
    scopes::check(&mut p.data, &mut p.database);
    // Count occurrences of the crate name — must be exactly 1.
    let count = p
        .data
        .native_packages
        .iter()
        .filter(|(c, _)| c == "loft-native-crate-test")
        .count();
    assert!(
        count <= 1,
        "P129: native_packages has {count} entries for loft-native-crate-test, expected at most 1"
    );
}

/// Regression: struct field types in use-loaded packages must resolve correctly.
/// Multiple structs + #native declarations + functions with return null.
#[test]
fn struct_fields_resolve_in_use_loaded_package() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}struct_order_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Struct field types should resolve in use-loaded packages; errors: {:?}",
        p.diagnostics.lines()
    );
}

/// Dep-shadowing regression: a package file named EXACTLY like a declared
/// dependency must not shadow that dependency in `use` resolution.
///
/// The package root sits in `lib_dirs` (that is what makes intra-package
/// `use otherfile;` work), so before the guard in `Parser::lib_path`,
/// `use shadowlib;` inside `consumer/shadowlib.loft` resolved to the file
/// itself: the real library never loaded, `shadowlib::Probe` came back
/// "Undefined type", and every qualified reference in the package errored.
/// This is how `tools/audience-demo/server.loft` (a package file named
/// `server.loft` next to a `server` dependency) silently broke.
///
/// Runs the real binary because the shadowing needs main.rs's package
/// detection (the walk-up that pushes the package root onto `lib_dirs`) —
/// an in-process `Parser` with hand-set `lib_dirs` cannot reproduce it.
#[test]
fn declared_dep_beats_same_named_package_file() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new(root.join("target/release/loft"))
        .args(["--interpret", "--no-warnings"])
        .arg(root.join("tests/fixtures/dep_shadow/consumer/shadowlib.loft"))
        .current_dir(&root)
        .output()
        .expect("run the dep_shadow consumer fixture");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("shadow-ok 42"),
        "dep-shadowing guard regressed: `use shadowlib;` did not resolve to \
         the declared dependency.  stdout={stdout:?} stderr={stderr:?}"
    );
}

/// #337 — PACKAGES.md resolution step 2: `use a;` resolves through the
/// consuming package's `[dependencies] a = { path = "…" }` even when the
/// dep is NOT a sibling package (previously only lib/, lib_dirs, and
/// sibling layouts worked; the compile-time resolver never consulted
/// path entries).
#[test]
fn i337_manifest_path_dep_resolves_non_sibling() {
    let tmp = std::env::temp_dir().join("loft_i337_pathdep");
    let _ = std::fs::remove_dir_all(&tmp);
    let dep_root = tmp.join("elsewhere").join("nested").join("a");
    std::fs::create_dir_all(dep_root.join("src")).unwrap();
    std::fs::write(
        dep_root.join("loft.toml"),
        "[package]\nname = \"a\"\nversion = \"0.0.1\"\n[library]\nentry = \"src/a.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dep_root.join("src").join("a.loft"),
        "pub fn a_hello() -> text { \"hello from a\" }\n",
    )
    .unwrap();
    let b_root = tmp.join("b");
    std::fs::create_dir_all(b_root.join("src")).unwrap();
    std::fs::write(
        b_root.join("loft.toml"),
        "[package]\nname = \"b\"\nversion = \"0.0.1\"\n[library]\nentry = \"src/b.loft\"\n\
         [dependencies]\na = { path = \"../elsewhere/nested/a\" }\n",
    )
    .unwrap();
    std::fs::write(
        b_root.join("src").join("b.loft"),
        "use a::*;\n\nfn main() {\n  log_info(\"{a_hello()}\");\n}\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&b_root.join("src").join("b.loft").to_string_lossy(), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "path dep should resolve; diagnostics: {:?}",
        p.diagnostics.lines()
    );
}

/// loft#714 — a `--lib` directory package's manifest dependencies must be pulled
/// in from the file that NAMED the package, not from wherever the descent had
/// reached by the time the queue was drained.
///
/// `switch_to_dep` replaces the lexer's source and the abandoned file resumes
/// later off `todo_files`. That is fine for a `use`, which always switches away
/// from the very file it appears in. Draining the manifest-dep queue from an
/// arbitrary descendant was not: a dep got pulled in while the lexer sat inside
/// an unrelated library whose definitions had not been parsed yet — and that
/// library was already `use_add`ed, so every later `use` of it was a no-op
/// against an empty library.
///
/// `pkg714_wrap` declares two deps whose graphs meet at `pkg714_c`; one dep alone
/// never reproduced it. The victim is a TUPLE destructure, because that is the
/// construct that needs the callee's return type at parse time — which is why
/// the reported failures read as `Unknown variable` and `Expect token ;` inside
/// published, CI-gated libraries rather than as anything about resolution.
#[test]
fn pkg_deps_resolve_before_the_dependent_is_parsed() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}pkg714_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "a directory package's two manifest deps must both resolve before the \
         libraries that use them are parsed; diagnostics: {:?}",
        p.diagnostics.lines()
    );
}

/// loft#826 — build a package whose entry `use`s one or more sibling files in
/// the same `src/`, and return the entry path.  Each `(name, body)` in
/// `siblings` becomes `src/<name>.loft`; the entry `use`s them in order.
fn i826_pkg(tag: &str, entry_body: &str, siblings: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("loft_i826_{tag}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("loft.toml"),
        "[package]\nname = \"pkg\"\nversion = \"0.0.1\"\n[library]\nentry = \"src/pkg.loft\"\n",
    )
    .unwrap();
    let uses: String = siblings
        .iter()
        .map(|(n, _)| format!("use {n}::*;\n"))
        .collect();
    std::fs::write(
        root.join("src").join("pkg.loft"),
        format!("{uses}{entry_body}"),
    )
    .unwrap();
    for (n, body) in siblings {
        std::fs::write(root.join("src").join(format!("{n}.loft")), body).unwrap();
    }
    root.join("src").join("pkg.loft")
}

fn i826_parse(entry: &std::path::Path) -> Vec<String> {
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&entry.to_string_lossy(), false);
    scopes::check(&mut p.data, &mut p.database);
    p.diagnostics.lines()
}

/// loft#826 (1) — a `use`d file that CALLS a function its importer declares is
/// refused, and the refusal must name the boundary.
///
/// The rule is real: `use helper;` imports helper's names into the entry, never
/// the entry's into helper, and helper is parsed BEFORE the entry reaches its
/// own definitions.  What made it cost minutes was the message — the fuzzy
/// same-name guess turned "`make` is out of scope here" into *"Unknown function
/// make — did you mean 'move'?"*, which points away from the cause while `make`
/// sits in the sibling file the author is looking at.
#[test]
fn i826_call_into_importer_names_the_boundary() {
    let entry = i826_pkg(
        "fn",
        "pub struct Thing { t_n: integer }\npub fn make() -> Thing { Thing { t_n: 1 } }\n",
        &[("helper", "pub fn via_fn() -> integer { make().t_n }\n")],
    );
    let d = i826_parse(&entry);
    let joined = d.join("\n");
    assert!(
        joined.contains("`make` is declared in pkg.loft:"),
        "the refusal must cite where `make` IS declared; got {d:?}"
    );
    assert!(
        joined.contains("never the other way round"),
        "the refusal must state which way a `use` carries names; got {d:?}"
    );
    assert!(
        !joined.contains("did you mean 'move'"),
        "a name declared by the importer is not a misspelling of an unrelated \
         keyword — the fuzzy guess must not outrank the boundary; got {d:?}"
    );
}

/// loft#826 (2) — TWO used files that each name a type their importer declares
/// must not be reported as two packages declaring it.
///
/// Each used file leaves a pub-visible `Unknown` forward-reference stub in its
/// own source, and those stubs import back into the entry like any other public
/// name.  The second landing on the first was recorded as a rival declaration,
/// so the entry's own `struct Thing` — declared once, on the line above — was
/// reported as `declared by more than one package`, advising a qualification
/// (`helper::Thing` or `second::Thing`) that names two files which declare no
/// such type and therefore cannot resolve either way.
#[test]
fn i826_two_used_files_are_not_a_multi_package_collision() {
    let entry = i826_pkg(
        "ambig",
        "pub struct Thing { t_n: integer }\npub fn make() -> Thing { Thing { t_n: 1 } }\n",
        &[
            ("helper", "pub fn bump(x: Thing) -> integer { x.t_n + 1 }\n"),
            (
                "second",
                "pub fn twice(x: Thing) -> integer { x.t_n * 2 }\n",
            ),
        ],
    );
    let d = i826_parse(&entry);
    let joined = d.join("\n");
    assert!(
        !joined.contains("declared by more than one package"),
        "a forward-reference stub declares nothing and must not count as a \
         rival declaration; got {d:?}"
    );
    assert!(
        !joined.contains("helper::Thing") && !joined.contains("second::Thing"),
        "no message may prescribe a qualification that cannot resolve; got {d:?}"
    );
    assert!(
        joined.contains("`Thing` is declared in pkg.loft:"),
        "the surviving refusal must cite the file that really declares it; got {d:?}"
    );
}

/// loft#826 — the cure the two messages above prescribe has to COMPILE, or they
/// prescribe an impossible fix.  Declarations both files need move into a third
/// file that each of them `use`s; types AND functions then cross, for two used
/// files at once.
#[test]
fn shared_sibling_carries_types_and_functions() {
    let entry = i826_pkg(
        "cure",
        "pub fn top() -> integer { bump(make()) + twice(make()) }\n",
        &[
            (
                "shared",
                "pub struct Thing { t_n: integer }\npub fn make() -> Thing { Thing { t_n: 1 } }\n",
            ),
            (
                "helper",
                "pub use shared::*;\npub fn bump(x: Thing) -> integer { x.t_n + 1 }\n",
            ),
            (
                "second",
                "pub use shared::*;\npub fn twice(x: Thing) -> integer { x.t_n * 2 }\n",
            ),
        ],
    );
    let d = i826_parse(&entry);
    assert!(
        !d.iter().any(|l| l.contains("rror")),
        "the prescribed cure must compile; diagnostics: {d:?}"
    );
}

/// A MUTUAL `use` is cured by importing the name: when this file also has a bare `use`
/// of the file that uses it, the two already `use` each other, and a mutual glob resolves
/// both ways (the p173 cycle) — no file moved.  dryopea's `spawn` (`use errand;`, a field
/// of type `Errand`) and `errand` (`use spawn::*;`) is the shape; the named cure is checked
/// to compile, so the note never prescribes one that does not.
#[test]
fn i826_mutual_use_names_the_import_cure() {
    let refused = i826_pkg(
        "mutual",
        "pub fn top() -> integer { spawn_n() }\n",
        &[
            (
                "spawn",
                "use errand;\npub struct Holder { route: Errand }\npub fn spawn_n() -> integer { 1 }\n",
            ),
            (
                "errand",
                "use spawn::*;\npub struct Errand { e_n: integer }\n\
                 pub fn errand_new() -> Errand { Errand { e_n: 4 } }\n",
            ),
        ],
    );
    let d = i826_parse(&refused).join("\n");
    assert!(
        d.contains("`use errand::*;`"),
        "the note names the import cure a mutual `use` allows:\n{d}"
    );
    let cured = i826_pkg(
        "mutual_cured",
        "pub fn top() -> integer { spawn_n() }\n",
        &[
            (
                "errand",
                "use spawn::*;\npub struct Errand { e_n: integer }\n\
                 pub fn errand_new() -> Errand { Errand { e_n: 4 } }\n",
            ),
            (
                "spawn",
                "use errand::*;\npub struct Holder { route: Errand }\n\
                 pub fn spawn_n() -> integer { 1 }\n",
            ),
        ],
    );
    let d = i826_parse(&cured);
    assert!(
        !d.iter().any(|l| l.contains("rror")),
        "the import cure must compile; diagnostics: {d:?}"
    );
}
