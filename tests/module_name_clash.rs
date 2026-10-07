// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A module's file NAME is shared across the whole dependency graph (loft#912).
//!
//! Two packages may each hold `src/catalogue.loft`, but only one of them can be
//! the module called `catalogue`: the second `use catalogue;` found the name
//! already taken and imported the OTHER package's file.  The loser's own
//! functions were then simply absent, reported as `Unknown function <name>` —
//! pointing at a line inside a package the author of the other one did not
//! write, may not have read, and cannot fix.  Nothing in the output said
//! "collision", so the search went looking for a missing `pub` or a typo.
//!
//! **The scoping fix has landed (loft#976).**  A bare `use <module>` inside a package now
//! binds THAT package's own `src/<module>.loft` when it ships one, registered under
//! `<package>::<module>` — which is what `use self::<module>` always did, and what the
//! advice below always recommended.  So a package's public surface no longer depends on
//! which siblings a consumer happens to pull, or in which order.
//!
//! The diagnostic stays, for the case the scoping rule cannot reach: a file with no
//! `<module>.loft` of its OWN still takes whichever the search finds, and two of those in
//! one graph still resolve by load order.  Its two tiers (loft#949) stay as they were —
//! `warning` when the ROOT PROJECT captures a name a DEPENDENCY was using, `advice` the
//! other way round.
//!
//! A DECLARED DEPENDENCY still beats a local file of the same name: that is `lib_path`'s
//! own shadow guard, deliberate, and the scoping rule defers to it — otherwise a package
//! holding `src/<dep>.loft` would stop being able to reach the `<dep>` it depends on.
//!
//! What the fix does NOT do is merge two modules into one name.  Where both packages'
//! modules declare the SAME public name and a consumer calls it BARE, that call is now an
//! explicit ambiguity error naming both — the missing error the pre-freeze mandate calls
//! for (COMPATIBILITY.md § the error surface is one-directional), and strictly better than
//! the arbitrary pick it replaces.

extern crate loft;

use loft::diagnostics::Level;
use loft::file_access as fa;
use loft::parser::Parser;

/// Build a two-package tree under a unique temp root:
///
/// ```text
/// pkg_dep/src/catalogue.loft   pub fn part_list()
/// pkg_dep/src/pkg_dep.loft     use catalogue;
/// pkg_top/src/<extra>          (the row under test)
/// pkg_top/src/pkg_top.loft     <top_body>
/// ```
///
/// `pkg_top` depends on `pkg_dep` by path.  Returns the parser's diagnostics
/// after parsing `pkg_top`'s entry file.
fn parse_two_packages(tag: &str, extra: &[(&str, &str)], top_body: &str) -> (Level, Vec<String>) {
    let root = std::env::temp_dir().join(format!("loft_912_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    fa::create_dir_all(dep.join("src")).expect("mkdir dep");
    fa::create_dir_all(top.join("src")).expect("mkdir top");

    fa::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 7 }\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/pkg_dep.loft"),
        "pub use catalogue::*;\npub fn dep_entry() -> integer { part_list() }\n",
    )
    .unwrap();

    fa::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    for (name, body) in extra {
        fa::write(top.join("src").join(name), body).unwrap();
    }
    fa::write(top.join("src/pkg_top.loft"), top_body).unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&top.join("src/pkg_top.loft").to_string_lossy(), false);
    let out = (p.diagnostics.level(), p.diagnostics.lines().to_vec());
    let _ = fa::remove_dir_all(&root);
    out
}

/// loft#976's own-module rule stops at the package's OWN NAME.
///
/// `use <pkg>` inside `<pkg>` asks for the package — its public surface — and that is how
/// every library in the ecosystem writes its own test suite: `tests/<pkg>.loft` containing
/// `use <pkg>;`. The rule as first written bound whatever file was called `<pkg>.loft`,
/// which for that suite is the TEST FILE, so the entry's `pub` surface never loaded and
/// every symbol read unknown. Nine published libraries went red on it — hex_world, glb,
/// regex, cbor, crypto, server, shapes, pluginabi, zttext — and the shape reduces to the
/// three files below.
///
/// The distinction is what lets a package refer to ITSELF: a name that means the package
/// in one file and a sibling module in another is a name that means nothing. `use
/// self::<pkg>` remains the explicit spelling for the file.
///
/// Built by hand rather than through `parse_two_packages`, because the whole point is a
/// file named after its own package in a directory that is not `src/`.
#[test]
fn a_packages_own_name_means_the_package_not_a_same_named_file() {
    let root = std::env::temp_dir().join(format!("loft_976_self_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("selfpkg");
    fa::create_dir_all(pkg.join("src")).expect("mkdir src");
    fa::create_dir_all(pkg.join("tests")).expect("mkdir tests");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"selfpkg\"\nversion = \"0.1.0\"\n\n\
         [library]\nentry = \"src/selfpkg.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/selfpkg.loft"),
        "pub fn from_the_entry() -> integer { 42 }\n",
    )
    .unwrap();
    // The file that used to win: named after the package, and NOT the entry.
    fa::write(
        pkg.join("tests/selfpkg.loft"),
        "use selfpkg::*;\nfn main() { assert(selfpkg::from_the_entry() == 42, \"entry surface\"); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("tests/selfpkg.loft").to_string_lossy(), false);
    let level = p.diagnostics.level();
    let lines = p.diagnostics.lines().to_vec();
    let _ = fa::remove_dir_all(&root);

    assert!(
        level < Level::Error,
        "`use selfpkg` inside package `selfpkg` must bind the PACKAGE, so the entry's \
         `pub` surface is what the test sees — binding the same-named test file instead \
         amputates it (loft#976): {lines:?}"
    );
}

/// The reported shape, and the one loft#976 fixes: the consumer grows a
/// `src/catalogue.loft` of its own while its dependency already has one.  Each `use
/// catalogue;` now binds the module of the package it is written in, so both packages
/// keep their own — the consumer's `top_only` AND the dependency's `part_list`.
#[test]
fn a_packages_own_module_wins_its_own_use() {
    let (level, diag) = parse_two_packages(
        "clash",
        &[("catalogue.loft", "pub fn top_only() -> integer { 3 }\n")],
        "pub use pkg_dep::*;\npub use catalogue::*;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    let all = diag.join("\n");
    assert!(
        !all.contains("Unknown function"),
        "neither package may lose its own module to the other — that amputation of a \
         published surface is what loft#912/#976 were about:\n{all}"
    );
    assert!(level < Level::Error, "and the tree must build:\n{all}");
    // Nothing left to advise about: the two modules are two keys, not one contested name.
    assert!(
        !all.contains("module 'catalogue' is declared by two files"),
        "a name each package resolves for itself is not a clash to report — a diagnostic \
         that fires where there is nothing to fix teaches people to skip the ones where \
         there is:\n{all}"
    );
}

/// The same tree with the two `use` lines REVERSED.  The whole defect was that this order
/// — the CONSUMER's, invisible to both library authors — decided which package lost its
/// module, so the two orders answering alike is the fix stated as a property.
#[test]
fn the_use_order_no_longer_decides_who_loses() {
    let (level_a, diag_a) = parse_two_packages(
        "order_a",
        &[("catalogue.loft", "pub fn top_only() -> integer { 3 }\n")],
        "pub use pkg_dep::*;\npub use catalogue::*;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    let (level_b, diag_b) = parse_two_packages(
        "order_b",
        &[("catalogue.loft", "pub fn top_only() -> integer { 3 }\n")],
        "pub use catalogue::*;\npub use pkg_dep::*;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    assert!(
        level_a < Level::Error && level_b < Level::Error,
        "both orders must build\nA:\n{}\nB:\n{}",
        diag_a.join("\n"),
        diag_b.join("\n")
    );
    assert!(
        !diag_a.join("\n").contains("Unknown function")
            && !diag_b.join("\n").contains("Unknown function"),
        "and neither may lose a module\nA:\n{}\nB:\n{}",
        diag_a.join("\n"),
        diag_b.join("\n")
    );
}

/// Control — a DIFFERENT basename is the documented fix, and it must still work.
/// This is the row that says the guard did not simply refuse intra-package
/// modules whose dependency happens to have modules of its own.
#[test]
fn a_distinct_basename_still_loads() {
    let (level, diag) = parse_two_packages(
        "renamed",
        &[("choices.loft", "pub fn top_only() -> integer { 3 }\n")],
        "pub use pkg_dep::*;\npub use choices::*;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() }\n",
    );
    assert!(
        !diag.join("\n").contains("declared by two files"),
        "a distinct name is not a clash:\n{}",
        diag.join("\n")
    );
    assert!(
        level < Level::Error,
        "renaming the module must leave a clean parse:\n{}",
        diag.join("\n")
    );
}

/// Control — ONE module `use`d from two files of the same package resolves to the
/// same file both times.  Refusing this would break every multi-file package, and
/// it is the shape a name-only check would get wrong.
#[test]
fn one_module_used_from_two_files_of_one_package_is_not_a_clash() {
    let (level, diag) = parse_two_packages(
        "twice",
        &[
            ("choices.loft", "pub fn top_only() -> integer { 3 }\n"),
            (
                "extra.loft",
                "pub use choices::*;\npub fn extra_fn() -> integer { top_only() + 1 }\n",
            ),
        ],
        "pub use choices::*;\npub use extra::*;\npub use pkg_dep::*;\n\
         pub fn top_entry() -> integer { pkg_dep::part_list() }\n\
         pub fn top_two() -> integer { top_only() + extra_fn() }\n",
    );
    assert!(
        !diag.join("\n").contains("declared by two files"),
        "the same file reached twice is one module, not two:\n{}",
        diag.join("\n")
    );
    assert!(
        level < Level::Error,
        "a multi-file package must still parse cleanly:\n{}",
        diag.join("\n")
    );
}

/// Control — a package file whose name matches a DECLARED DEPENDENCY is already
/// governed by the dep-shadowing guard in `lib_path`: the dependency wins, on
/// purpose.  The clash check must defer to that rather than report a second,
/// contradictory verdict on the same name.
#[test]
fn a_file_named_like_a_declared_dependency_is_not_a_clash() {
    let (level, diag) = parse_two_packages(
        "shadow",
        &[("pkg_dep.loft", "pub fn shadow_fn() -> integer { 99 }\n")],
        "pub use pkg_dep::*;\npub fn top_entry() -> integer { pkg_dep::part_list() }\n",
    );
    assert!(
        !diag.join("\n").contains("declared by two files"),
        "a declared dependency name is resolved by the shadow guard, not refused:\n{}",
        diag.join("\n")
    );
    assert!(
        level < Level::Error,
        "the shadow guard's own resolution must stay clean:\n{}",
        diag.join("\n")
    );
}

/// loft#948 — the shadowing file declares something ELSE, so the collision BREAKS the build.
///
/// This is the case #912 was filed about and the only one a person cannot work out from the
/// output: the errors name a line inside a DEPENDENCY the consumer never edited, the missing
/// function is `pub`, the dependency is green on its own, and the cure — rename your own new
/// file — follows from nothing printed.
///
/// loft#948's scaffold, which is the FATAL shape: the consumer's `src/catalogue.loft`
/// declares something else entirely and is imported by nobody, so the dependency's
/// `part_list` simply went missing and the package's own test suite failed.
///
/// Driven through the BINARY rather than `Parser::parse`, and that is load-bearing: the
/// shadowing file is never imported, so nothing reaches it by following `use` edges. It is
/// loaded because building the package reads every file under `src/` — which is why the
/// collision happened at all.
///
/// Under loft#976 there is no collision left: `pkg_dep` binds its own `catalogue` and the
/// consumer's same-named file is simply a different module.
#[test]
fn the_collision_no_longer_breaks_the_build() {
    let root = std::env::temp_dir().join(format!("loft_948_fatal_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    fa::create_dir_all(dep.join("src")).expect("mkdir dep");
    fa::create_dir_all(top.join("src")).expect("mkdir top");
    fa::create_dir_all(top.join("tests")).expect("mkdir tests");

    fa::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 41 }\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/pkg_dep.loft"),
        "use catalogue::*;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    )
    .unwrap();

    fa::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    fa::write(
        top.join("src/pkg_top.loft"),
        "use pkg_dep::*;\npub fn top_answer() -> integer { dep_answer() }\n",
    )
    .unwrap();
    // Declares something else entirely and is imported by nobody, so the dependency's
    // `part_list` simply goes missing.
    fa::write(
        top.join("src/catalogue.loft"),
        "pub fn top_unrelated() -> text { \"x\" }\n",
    )
    .unwrap();
    fa::write(
        top.join("tests/answer.loft"),
        "use pkg_top::*;\nfn main() { assert(top_answer() == 42, \"answer\"); }\n",
    )
    .unwrap();

    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = loft::platform::process::harness_command(bin.join("loft"))
        .arg("test")
        .current_dir(&top)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("invoke loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);

    assert!(
        !all.contains("Unknown function part_list"),
        "loft#976 — `pkg_dep`'s `use catalogue::*` binds ITS OWN catalogue, so the \
         consumer's unrelated file of that name can no longer take `part_list` away from \
         it. The dependency answers 42 here exactly as it does alone:\n{all}"
    );
    assert!(
        all.contains("1 passed") || all.contains("test result: ok"),
        "…and the package's own test suite therefore passes:\n{all}"
    );
}

/// loft#948 — two files of ONE package are not this collision, so they must stay quiet.
///
/// `tests/<pkg>.loft` beside `src/<pkg>.loft` is an ordinary layout — two of this repo's own
/// fixtures use it — and the `use` binds the one the author meant. The advice is about a name
/// "shared across the whole dependency graph"; firing it where there is nothing to fix is how
/// a diagnostic teaches people to skip the ones where there is.
#[test]
fn a_same_package_basename_collision_is_not_advised() {
    let root = std::env::temp_dir().join(format!("loft_948_same_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("pkg_solo");
    fa::create_dir_all(pkg.join("src")).expect("mkdir src");
    fa::create_dir_all(pkg.join("tests")).expect("mkdir tests");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg_solo\"\nversion = \"0.1.0\"\nentry = \"src/pkg_solo.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/pkg_solo.loft"),
        "pub fn solo_answer() -> integer { 42 }\n",
    )
    .unwrap();
    // Named after its own package — the shape that used to draw a rename it must not draw.
    fa::write(
        pkg.join("tests/pkg_solo.loft"),
        "use pkg_solo::*;\nfn main() { assert(solo_answer() == 42, \"solo\"); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("tests/pkg_solo.loft").to_string_lossy(), false);
    let lines = p.diagnostics.lines().to_vec();
    let _ = fa::remove_dir_all(&root);

    assert!(
        !lines.iter().any(|l| l.contains("module-name-shadowed")),
        "two files of ONE package drew a cross-package collision advice:\n{lines:#?}"
    );
}

/// Build the loft#949 tree and return what `loft test` prints for a given `pkg_dep`
/// entry body.  The consumer ships its own `src/catalogue.loft` declaring `part_list`
/// with the SAME signature as the dependency's, so nothing errors — the dependency
/// simply answers with the consumer's number.
///
/// Driven through the BINARY, and that is load-bearing: nobody imports the consumer's
/// `catalogue.loft`, so no `use` edge reaches it.  It is loaded because building a
/// package reads every file under `src/` — which is what lets a consumer take a name
/// its dependency has not asked for yet.
fn use_self_tree(tag: &str, dep_entry: &str) -> String {
    let root = std::env::temp_dir().join(format!("loft_949_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    fa::create_dir_all(dep.join("src")).expect("mkdir dep");
    fa::create_dir_all(top.join("src")).expect("mkdir top");
    fa::create_dir_all(top.join("tests")).expect("mkdir tests");

    fa::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 41 }\n",
    )
    .unwrap();
    fa::write(dep.join("src/pkg_dep.loft"), dep_entry).unwrap();

    fa::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    fa::write(
        top.join("src/pkg_top.loft"),
        "use pkg_dep::*;\npub fn top_answer() -> integer { dep_answer() }\n",
    )
    .unwrap();
    // Same name, same signature — so this does not break the build, it changes the answer.
    fa::write(
        top.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 99 }\n",
    )
    .unwrap();
    fa::write(
        top.join("tests/answer.loft"),
        "use pkg_top::*;\nfn main() { println(\"answer={top_answer()}\"); }\n",
    )
    .unwrap();

    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = loft::platform::process::harness_command(bin.join("loft"))
        .arg("test")
        .current_dir(&top)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_AUTO_INSTALL", "1")
        .output()
        .expect("invoke loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);
    all
}

/// loft#949/#976 — a dependency answers the same in every consumer, and BOTH spellings
/// now say so.
///
/// `use self::<module>` always bound the package's own module; since loft#976 a bare `use
/// <module>` inside a package means the same thing, so the two arms must agree. They are
/// both run because the pair is the claim: one arm alone could be green on a tree that
/// stopped colliding for some unrelated reason.
#[test]
fn use_self_binds_the_packages_own_module_not_a_consumers() {
    let bare = use_self_tree(
        "bare",
        "use catalogue::*;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    );
    assert!(
        bare.contains("answer=42"),
        "loft#976 — a bare `use catalogue` inside pkg_dep binds pkg_dep's own \
         catalogue.loft (41 + 1), whatever the consumer ships. Reading 100 here means the \
         consumer's file captured the name again:\n{bare}"
    );

    let scoped = use_self_tree(
        "self",
        "pub use self::catalogue::*;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    );
    assert!(
        scoped.contains("answer=42"),
        "`use self::catalogue` must bind pkg_dep's own catalogue.loft (41 + 1), \
         whatever the consumer ships:\n{scoped}"
    );
    // No collision left to report: the module is registered under `pkg_dep::catalogue`,
    // so the consumer's `catalogue` is no longer competing for the same slot.
    assert!(
        !scoped.contains("module-name-shadowed"),
        "a scoped module must not still be reported as sharing a name:\n{scoped}"
    );
}

/// The half "prefer the local file" could not have delivered: two packages' same-named
/// modules COEXIST, each declaring its own `struct Row` and its own `make_row`, and each
/// answers its own.
///
/// A precedence rule alone cannot do this — `use_names` is a flat map and `use_add`
/// derives the source id from its size, so one `catalogue` overwrites the other however
/// the winner is chosen. `self::` registers under `<package>::<module>`, which is two
/// keys, so both stay reachable and the database type keys stay distinct too.
///
/// The values are the assertion, not the absence of an error: `dep_tag=dep` can only come
/// from pkg_dep's `Row`, `top_label=top` only from pkg_top's, and they have different
/// FIELDS — so a schema that had merged them could not produce both.
#[test]
fn two_packages_same_named_self_modules_both_stay_reachable() {
    let root = std::env::temp_dir().join(format!("loft_949_coexist_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let dep = root.join("pkg_dep");
    let top = root.join("pkg_top");
    fa::create_dir_all(dep.join("src")).expect("mkdir dep");
    fa::create_dir_all(top.join("src")).expect("mkdir top");
    fa::create_dir_all(top.join("tests")).expect("mkdir tests");

    fa::write(
        dep.join("loft.toml"),
        "[package]\nname = \"pkg_dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_dep.loft\"\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/catalogue.loft"),
        "pub struct Row { pub n: integer, pub tag: text }\n\
         pub fn make_row() -> Row { Row { n: 41, tag: \"dep\" } }\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/pkg_dep.loft"),
        "pub use self::catalogue::*;\n\
         pub fn dep_answer() -> integer { make_row().n + 1 }\n\
         pub fn dep_tag() -> text { make_row().tag }\n",
    )
    .unwrap();

    fa::write(
        top.join("loft.toml"),
        "[package]\nname = \"pkg_top\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/pkg_top.loft\"\n\n\
         [dependencies]\npkg_dep = { path = \"../pkg_dep\" }\n",
    )
    .unwrap();
    // Same module name, same function name, a DIFFERENT struct behind it.
    fa::write(
        top.join("src/catalogue.loft"),
        "pub struct Row { pub label: text, pub extra: float }\n\
         pub fn make_row() -> Row { Row { label: \"top\", extra: 2.5 } }\n",
    )
    .unwrap();
    fa::write(
        top.join("src/pkg_top.loft"),
        "pub use pkg_dep::*;\nuse self::catalogue as m;\n\
         pub fn top_answer() -> integer { dep_answer() }\n\
         pub fn top_label() -> text { m::make_row().label }\n",
    )
    .unwrap();
    fa::write(
        top.join("tests/answer.loft"),
        "use pkg_top::*;\n\
         fn main() { println(\"dep_answer={top_answer()} dep_tag={dep_tag()} top_label={top_label()}\"); }\n",
    )
    .unwrap();

    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = loft::platform::process::harness_command(bin.join("loft"))
        .arg("test")
        .current_dir(&top)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_AUTO_INSTALL", "1")
        .output()
        .expect("invoke loft test");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);

    assert!(
        all.contains("dep_answer=42 dep_tag=dep top_label=top"),
        "both packages' `catalogue` modules must stay reachable, each answering its \
         own:\n{all}"
    );
}

/// `use self::<m>` accepts the same import spec as `use <lib>` — the grammar is shared,
/// so the two spellings cannot drift into accepting different things.
///
/// The refused case is the point of the last row: the flat comma list is rejected for
/// `self::` too, and the message quotes the spelling the author actually wrote
/// (`use self::tools::(a, b, …)`) rather than a bare-library one they did not.
#[test]
fn use_self_takes_the_same_import_spec_as_a_library() {
    let root = std::env::temp_dir().join(format!("loft_949_spec_{}", std::process::id()));
    let pkg = root.join("pkg");
    for (tag, body, want) in [
        ("pub", "pub use self::tools::*;", "1|2"),
        ("one", "use self::tools::one;", "1|"),
        ("group", "use self::tools::(one, two);", "1|2"),
        ("rename", "use self::tools::(one as first);", "1|"),
        ("star", "use self::tools::*;", "1|2"),
    ] {
        let _ = fa::remove_dir_all(&root);
        fa::create_dir_all(pkg.join("src")).expect("mkdir");
        fa::write(
            pkg.join("loft.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nentry = \"src/pkg.loft\"\n",
        )
        .unwrap();
        fa::write(
            pkg.join("src/tools.loft"),
            "pub fn one() -> integer { 1 }\npub fn two() -> integer { 2 }\n",
        )
        .unwrap();
        // `first` is bound only in the rename row; every row prints what it imported.
        let call = if tag == "rename" { "first()" } else { "one()" };
        let second = if want.ends_with('2') { "two()" } else { "\"\"" };
        fa::write(
            pkg.join("src/pkg.loft"),
            format!("{body}\nfn main() {{ println(\"{{{call}}}|{{{second}}}\") }}\n"),
        )
        .unwrap();

        let mut p = Parser::new();
        p.parse_dir("default", true, true).unwrap();
        p.parse(&pkg.join("src/pkg.loft").to_string_lossy(), false);
        assert!(
            p.diagnostics.level() < Level::Error,
            "`{body}` must parse:\n{}",
            p.diagnostics.lines().join("\n")
        );
    }

    // The flat comma list is refused for `self::` exactly as it is for a library.
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(pkg.join("src")).expect("mkdir");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nentry = \"src/pkg.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/tools.loft"),
        "pub fn one() -> integer { 1 }\npub fn two() -> integer { 2 }\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/pkg.loft"),
        "use self::tools::one, two;\nfn main() { println(\"{one()}\") }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/pkg.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = fa::remove_dir_all(&root);
    assert!(
        all.contains("use self::tools::(a, b, …)"),
        "the grouping error must quote the spelling the author wrote:\n{all}"
    );

    // A bare `use self::tools;` would bind nothing: a bare `use` brings in only a
    // qualifier (@C98), and a package's own module has none (loft#976).  It is refused,
    // naming the three spellings that do bind something.
    fa::create_dir_all(pkg.join("src")).expect("mkdir");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nentry = \"src/pkg.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/tools.loft"),
        "pub fn one() -> integer { 1 }\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/pkg.loft"),
        "use self::tools;\nfn main() { println(\"{tools::one()}\") }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/pkg.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = fa::remove_dir_all(&root);
    assert!(
        all.contains("`use self::tools;` binds nothing")
            && all.contains("use self::tools::*;")
            && all.contains("use self::tools as <alias>;"),
        "a bare `use self::` is refused with its cures:\n{all}"
    );
}

/// `use self::<module>` must NOT fall through to the wider `lib_path` search when the
/// module is absent.
///
/// Bare `use` searches outward by design — project `lib/`, sibling packages, the
/// registry. Letting `self::` do the same would mean a typo silently binds some other
/// package's file, which is the exact outcome the spelling exists to prevent. So an
/// absent module is an error that names the package it looked in.
#[test]
fn use_self_refuses_to_search_outside_its_own_package() {
    let root = std::env::temp_dir().join(format!("loft_949_absent_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("pkg_solo");
    fa::create_dir_all(pkg.join("src")).expect("mkdir src");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"pkg_solo\"\nversion = \"0.1.0\"\nentry = \"src/pkg_solo.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/pkg_solo.loft"),
        "pub use self::nope::*;\npub fn solo() -> integer { 1 }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/pkg_solo.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = fa::remove_dir_all(&root);

    assert_eq!(
        p.diagnostics.level(),
        Level::Error,
        "an absent self-module must be an error, not a silent outward search:\n{all}"
    );
    assert!(
        all.contains("pkg_solo") && all.contains("nope"),
        "the error must name the package it searched and the module it wanted:\n{all}"
    );
}

/// `self` needs a package to mean something.  In a bare script there is no
/// `[package] name` to qualify with, so the spelling is refused — and the message
/// carries the two ways forward rather than only the refusal.
#[test]
fn use_self_outside_a_package_says_what_to_do_instead() {
    let root = std::env::temp_dir().join(format!("loft_949_nopkg_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(&root).expect("mkdir root");
    fa::write(
        root.join("helper.loft"),
        "pub fn helper() -> integer { 1 }\n",
    )
    .unwrap();
    fa::write(
        root.join("bare.loft"),
        "use self::helper::*;\nfn main() { println(\"{helper()}\") }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&root.join("bare.loft").to_string_lossy(), false);
    let all = p.diagnostics.lines().join("\n");
    let _ = fa::remove_dir_all(&root);

    assert_eq!(
        p.diagnostics.level(),
        Level::Error,
        "expected an error:\n{all}"
    );
    assert!(
        all.contains("loft.toml") && all.contains("use helper;"),
        "the message must name both cures — add a manifest, or take the module by its \
         shared name:\n{all}"
    );
}

/// The other side of that refusal: where `self::` is unavailable, the clash advice must
/// not prescribe it (loft#949).
///
/// `use self::<id>` qualifies with `[package] name`, so a bare script has nothing to
/// qualify with and the spelling is refused — the test above pins that. An advice that
/// recommended it anyway would hand the reader a cure that errors, which is the failure
/// the `self::` work set out to avoid in its own ambiguity message. So the cure named
/// here is keyed to whether the file HAS a package, and a bare script hears the rename.
#[test]
fn the_clash_advice_outside_a_package_does_not_prescribe_self() {
    let root = std::env::temp_dir().join(format!("loft_949_advice_nopkg_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let a = root.join("dir_a");
    let b = root.join("dir_b");
    fa::create_dir_all(&a).expect("mkdir a");
    fa::create_dir_all(&b).expect("mkdir b");
    // Neither directory carries a `loft.toml`, so neither file is in a package.
    fa::write(
        a.join("catalogue.loft"),
        "pub fn a_only() -> integer { 1 }\n",
    )
    .unwrap();
    fa::write(
        b.join("catalogue.loft"),
        "pub fn b_only() -> integer { 2 }\n",
    )
    .unwrap();
    fa::write(
        b.join("bare.loft"),
        "use catalogue::*;\nfn main() { println(\"{b_only()}\") }\n",
    )
    .unwrap();

    // Driven through the BINARY: the clash needs `catalogue` to resolve OUTWARD to
    // dir_a, which is what `--lib` sets up and what an in-process `Parser` has no
    // search path for.  Without it the `use` binds dir_b's own file, there is no
    // clash, and the test would assert nothing — which is how it first failed.
    let mut bin = std::env::current_exe().expect("test binary path");
    bin.pop();
    if bin.ends_with("deps") {
        bin.pop();
    }
    let out = loft::platform::process::harness_command(bin.join("loft"))
        .args(["--interpret", "--path", env!("CARGO_MANIFEST_DIR"), "--lib"])
        .arg(&a)
        .arg(b.join("bare.loft"))
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("invoke loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);

    assert!(
        all.contains("module 'catalogue' is declared by two files"),
        "the scaffold must still produce the clash, or this asserts nothing:\n{all}"
    );
    assert!(
        !all.contains("use self::"),
        "a bare script cannot spell `use self::` — recommending it prescribes an error:\n{all}"
    );
    assert!(
        all.contains("Rename one file"),
        "so it must hear the cure that does work for it:\n{all}"
    );
}

/// The value loft#949 was about, now from the other side: a dependency that is published,
/// versioned and green on its own answers THE SAME once a consumer adds a file whose
/// basename it was already using.  Nothing in the dependency changed, the consumer never
/// imported the dependency's module, and the two files share only a name — so the two must
/// not share a slot.
///
/// Both numbers come from ONE run, which is what makes it a comparison rather than an
/// anecdote: `dep_answer()` reads 42 through the dependency's own `part_list` while the
/// consumer's own call reads its own 99.  Before loft#976 the first read 100 — the
/// dependency running on someone else's data.
#[test]
fn a_dependency_answers_the_same_in_every_consumer() {
    // The consumer's module declares its OWN name, so the only question this cell asks is
    // whose `catalogue` the DEPENDENCY read. (Both declaring `part_list` is the next test.)
    let all = captured_module_run(
        "value",
        "pub fn con_list() -> integer { 99 }\n",
        "use catalogue::*;\nuse dep::*;\n\
         fn main() { println(\"dep={dep_answer()} con={con_list()}\"); }\n",
    );
    assert!(
        all.contains("dep=42 con=99"),
        "the dependency must answer 42 — what it answers alone — while the consumer's own \
         `part_list` still answers 99. `dep=100` is the captured-module wrong result \
         (loft#949/#976):\n{all}"
    );
    assert!(
        !all.contains("module-name-shadowed"),
        "and there is nothing left to warn about: each package resolved its own file:\n{all}"
    );
}

/// The shape the scoping rule deliberately does NOT merge: both packages' modules declare
/// the SAME public name, and the consumer calls it BARE with both in scope.
///
/// That call has two answers and no rule picks between them, so it is an ERROR naming both
/// — not the arbitrary pick it used to be. Adding it is the pre-freeze mandate for a
/// surface that "produces a plausible-wrong value where it should reject"
/// (COMPATIBILITY.md § the error surface is one-directional); the qualified spellings the
/// message names both keep working.
#[test]
fn a_bare_call_matching_two_modules_is_refused_not_guessed() {
    let all = captured_module_run(
        "ambig",
        "pub fn part_list() -> integer { 99 }\n",
        "use catalogue::*;\nuse dep::*;\n\
         fn main() { println(\"{part_list()}\"); }\n",
    );
    assert!(
        all.contains("declared by more than one module"),
        "a bare name two modules answer must be refused, not resolved by load order:\n{all}"
    );
    assert!(
        all.contains("con::catalogue::part_list") && all.contains("dep::catalogue::part_list"),
        "…and the message must name BOTH, since the reader's fix is to pick one:\n{all}"
    );
    assert!(
        !all.contains("A module taken with `use self::`"),
        "the message must not assume the reader wrote `self::` — since loft#976 a bare \
         `use` inside a package scopes the same way:\n{all}"
    );
    // One source is now reachable under TWO names — `con::catalogue` and the short
    // `catalogue` qualifier — and the name the message picks came from a HashMap walk, so
    // it varied run to run. A diagnostic that renames the thing it is about between runs
    // is a bug in the diagnostic, and a test that only ran once would report it as a flake.
    let again = captured_module_run(
        "ambig2",
        "pub fn part_list() -> integer { 99 }\n",
        "use catalogue::*;\nuse dep::*;\n\
         fn main() { println(\"{part_list()}\"); }\n",
    );
    assert_eq!(
        all.replace("ambig_", "T_").replace("ambig2_", "T_"),
        again.replace("ambig_", "T_").replace("ambig2_", "T_"),
        "the same program must name the same definitions every run"
    );
}

/// loft#976's own shape: two SIBLING packages, neither depending on the other, with
/// DISJOINT names inside the colliding module — and a consumer that pulls both.
///
/// This is the one a precedence rule between a package and its dependency cannot reach.
/// Each package's own test suite was green, because a package's own graph holds only
/// itself; the loser was decided by the CONSUMER's `use` order, which neither author can
/// see, and a qualified `pkg::name` could not rescue it — the module never loaded, so
/// there was no second name to choose between. Both orders are run, because the order
/// being irrelevant IS the fix.
#[test]
fn two_sibling_packages_keep_their_own_same_named_modules() {
    for (tag, uses) in [
        ("ab", "pub use pkg_a::*;\npub use pkg_b::*;\n"),
        ("ba", "pub use pkg_b::*;\npub use pkg_a::*;\n"),
    ] {
        let root = std::env::temp_dir().join(format!("loft_976_{tag}_{}", std::process::id()));
        let _ = fa::remove_dir_all(&root);
        for (pkg, fname, body, entry) in [
            (
                "pkg_a",
                "skin.loft",
                "pub fn skin_a_only(v: float) -> float { v * 2.0 }\n",
                "pub use skin::*;\npub fn a_entry(v: float) -> float { skin_a_only(v) }\n",
            ),
            (
                "pkg_b",
                "skin.loft",
                "pub fn skin_b_only(v: float) -> float { v + 10.0 }\n",
                "pub use skin::*;\npub fn b_entry(v: float) -> float { skin_b_only(v) }\n",
            ),
        ] {
            let dir = root.join(pkg);
            fa::create_dir_all(dir.join("src")).expect("mkdir pkg");
            fa::write(
                dir.join("loft.toml"),
                format!(
                    "[package]\nname = \"{pkg}\"\nversion = \"0.1.0\"\n\n\
                     [library]\nentry = \"src/{pkg}.loft\"\n"
                ),
            )
            .unwrap();
            fa::write(dir.join("src").join(fname), body).unwrap();
            fa::write(dir.join("src").join(format!("{pkg}.loft")), entry).unwrap();
        }
        let app = root.join("app");
        fa::create_dir_all(app.join("src")).expect("mkdir app");
        fa::write(
            app.join("loft.toml"),
            "[package]\nname = \"app976\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
             pkg_a = { path = \"../pkg_a\" }\npkg_b = { path = \"../pkg_b\" }\n",
        )
        .unwrap();
        fa::write(
            app.join("src/main.loft"),
            format!(
                "{uses}fn main() {{ \
                 println(\"a={{pkg_a::a_entry(1.0)}} b={{pkg_b::b_entry(1.0)}}\") }}\n"
            ),
        )
        .unwrap();

        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args(["--interpret", "src/main.loft"])
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "120")
            .current_dir(&app)
            .output()
            .expect("spawn loft");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = fa::remove_dir_all(&root);

        assert!(
            all.contains("a=2 b=11"),
            "[{tag}] both packages must reach their own `skin` — before loft#976 the \
             second `use` found the name taken and that package's module never loaded, \
             so its public surface was amputated in a build it had nothing to do \
             with:\n{all}"
        );
    }
}

/// Build `dep` (its own `catalogue` answering 41) + `con` (a `catalogue` of its own, and
/// `dep` as a path dependency), run `con`'s main, and return everything it said.
fn captured_module_run(tag: &str, con_catalogue: &str, con_main: &str) -> String {
    let root = std::env::temp_dir().join(format!("loft_949_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let dep = root.join("dep");
    let con = root.join("con");
    fa::create_dir_all(dep.join("src")).expect("mkdir dep");
    fa::create_dir_all(con.join("src")).expect("mkdir con");

    fa::write(
        dep.join("loft.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/dep.loft\"\n",
    )
    .unwrap();
    // 41 + 1 = 42 when the dependency reads its OWN catalogue.
    fa::write(
        dep.join("src/dep.loft"),
        "pub use catalogue::*;\npub fn dep_answer() -> integer { part_list() + 1 }\n",
    )
    .unwrap();
    fa::write(
        dep.join("src/catalogue.loft"),
        "pub fn part_list() -> integer { 41 }\n",
    )
    .unwrap();

    fa::write(
        con.join("loft.toml"),
        "[package]\nname = \"con\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
         dep = { path = \"../dep\" }\n",
    )
    .unwrap();
    fa::write(con.join("src/catalogue.loft"), con_catalogue).unwrap();
    fa::write(con.join("src/main.loft"), con_main).unwrap();

    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "src/main.loft"])
        // No program cache: a directory that has been run before under a different
        // arrangement of `use` lines answers from the cache, and the cached answer is the
        // OTHER cell of this matrix.
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120")
        .current_dir(&con)
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);
    all
}

/// loft#1043 — qualifying a `self::`-bound module by its SHORT name is refused, and
/// the refusal has to say why rather than calling the module unknown.
///
/// `use self::<m>;` registers the module under `<pkg>::<m>` and deliberately does not
/// take the flat `<m>::` qualifier slot — that slot is shared by the whole dependency
/// graph, and staying out of it is exactly what the `self::` spelling is for
/// (loft#976). So the module is bound and reachable BARE, and only the qualifier is
/// absent. Reporting `Unknown library 'm'` states the opposite, and the cascading
/// `Unknown variable` errors on the lines after it read as *that module is missing*.
#[test]
fn qualifying_a_self_bound_module_explains_itself() {
    let root = std::env::temp_dir().join(format!("loft_1043_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("qpkg");
    fa::create_dir_all(pkg.join("src")).expect("mkdir src");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"qpkg\"\nversion = \"0.1.0\"\n\n\
         [library]\nentry = \"src/qpkg.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/surfaces.loft"),
        "pub fn count() -> integer { 7 }\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/qpkg.loft"),
        "pub use self::surfaces::*;\npub fn n() -> integer { surfaces::count() }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/qpkg.loft").to_string_lossy(), false);
    let lines = p.diagnostics.lines().to_vec();
    let _ = fa::remove_dir_all(&root);
    let all = lines.join("\n");

    assert!(
        all.contains("this package's own module"),
        "the refusal must recognise it as this package's own module: {lines:?}"
    );
    assert!(
        all.contains("withheld on purpose"),
        "…and say the qualifier is withheld rather than missing: {lines:?}"
    );
    assert!(
        all.contains("as <alias>"),
        "…and name the spelling that does give a qualifier: {lines:?}"
    );
    // ⚠ The alias re-enters the shared slot, so advising it without that caveat would
    // hand back the capture `self::` exists to prevent.
    assert!(
        all.contains("shared slot"),
        "…including that an alias takes the shared slot: {lines:?}"
    );
    assert!(
        !all.contains("Unknown library"),
        "and it must NOT call a module that is right there unknown: {lines:?}"
    );
}

/// THE CONTROL: a qualifier that names nothing at all must still report the plain
/// unknown-library error. If this ever produced the own-module message, the new
/// diagnostic would have stopped distinguishing the two.
#[test]
fn a_genuinely_unknown_library_still_says_unknown() {
    let root = std::env::temp_dir().join(format!("loft_1043c_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("qpkg");
    fa::create_dir_all(pkg.join("src")).expect("mkdir src");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"qpkg\"\nversion = \"0.1.0\"\n\n\
         [library]\nentry = \"src/qpkg.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src/qpkg.loft"),
        "pub fn n() -> integer { nosuchlib::count() }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse(&pkg.join("src/qpkg.loft").to_string_lossy(), false);
    let lines = p.diagnostics.lines().to_vec();
    let _ = fa::remove_dir_all(&root);
    let all = lines.join("\n");

    assert!(
        all.contains("Unknown library 'nosuchlib'"),
        "a name that is no module and no library is still unknown: {lines:?}"
    );
    assert!(
        !all.contains("own module"),
        "and must not be dressed up as this package's own: {lines:?}"
    );
}
