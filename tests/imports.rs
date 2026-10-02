// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Integration tests for T1-2: wildcard and selective imports.
//! Verifies that `use mylib::*` and `use mylib::name` bring library names
//! into scope without a qualifier.

extern crate loft;

use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::platform::sep_str;
use loft::scopes;

/// `use importlib::*` makes all names (add, mul, Point) directly accessible.
#[test]
fn wildcard_import_makes_names_accessible() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}wildcard_import_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// `use importlib::add` makes only `add` directly accessible; mul and Point are not imported.
#[test]
fn selective_import_makes_named_item_accessible() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}selective_import_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// `use importlib::nope` where `nope` does not exist in importlib produces an error.
#[test]
fn selective_import_of_unknown_name_is_error() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}bad_import_main.loft"), false);
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected an error for importing nonexistent name 'nope'"
    );
}

/// C53: match arms accept bare and qualified library enum variants.
#[test]
fn match_accepts_library_enum_variants() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}match_lib_enum_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// P173: two files that `use` each other (cyclic intra-package import)
/// must resolve both sides' public types so every cross-file reference
/// links to the real definition.  Before the P173 fix this failed with
/// "Undefined type TypeA" / "Undefined type TypeB" because `use X;` queued
/// an import that was applied before X's definitions were registered.
///
/// The fix: `parse_file` runs `actual_types_deferred` with a buffer that
/// collects unresolved stubs; after the full recursion (and a round of
/// `import_all_overwrite`), `resolve_deferred_unknowns` patches the stubs
/// to their real definitions via `rewrite_unknown_refs`.
#[test]
fn p173_intra_cycle_resolves_cross_file_types() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p173_cycle_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected cyclic `use` to resolve; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN22 Phase 3 — `use … as …` aliasing.  Parses a main file that uses all
/// three alias forms against `enumlib` (library alias `use enumlib as el`, type
/// alias `use enumlib::Status as St`, function alias `use enumlib::make as mk`)
/// and asserts every alias binds (an unbound alias surfaces as "Unknown
/// function" / "Undefined type").
#[test]
fn pln22_phase3_use_as_aliasing() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p3_alias_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN22 Phase 4 — grouped selective import `use lib::(a as x, b);`.  Parses a
/// main file that imports two names from `enumlib` in one parenthesised group,
/// with per-name aliases, and asserts both bind.
/// @C76 — selective imports group with `()`.
#[test]
fn pln22_phase4_grouped_import() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p4_group_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "Expected no errors; got: {:?}",
        p.diagnostics.lines()
    );
}

/// @PLN22 Phase 4 — the flat comma list `use lib::a, b` is dropped; multiple
/// names must be parenthesised.  Parsing the flat form must produce an error.
/// @C76 — the flat comma list is refused.
#[test]
fn pln22_phase4_flat_list_rejected() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}p4_flat_rejected.loft"), false);
    assert!(
        p.diagnostics.level() >= Level::Error,
        "Expected the flat `use lib::a, b` list to be rejected"
    );
    // The Libraries chapter quotes this sentence, so the reader who wrote the flat list
    // recognises the answer they get.  A refusal the reference describes in its own words
    // is a promise about a string, and only a substring match keeps it.
    let said = p.diagnostics.lines().join("\n");
    assert!(
        said.contains("import multiple names with parentheses"),
        "the refusal must still say what the Libraries chapter quotes; got: {said}"
    );
}

/// @PLN102 C97 — a library may define a name that also exists in the stdlib (`clamp`);
/// it is module-scoped (reached as `c97_shadowlib::clamp`) and does NOT trigger the C95
/// "Cannot redefine" error, while the bare name stays the stdlib's.  This is the fix that
/// lets the stdlib grow without breaking a shipped library (the shapes/time break).
/// @C97 — a library's public symbols live under its module.
#[test]
fn pln102_c97_library_may_define_a_stdlib_name() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}c97_shadow_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "a library defining the stdlib name `clamp` must be module-scoped, not a C95 redefinition; got: {:?}",
        p.diagnostics.lines()
    );
}

/// loft#940 — C97 keeps the definition legal, and says so out loud.
///
/// The C97 test above asserts the half that must not error. This asserts the half that
/// must not be SILENT: the author wrote a function, every bare call goes somewhere else,
/// and until now nothing said which. Pinned on the same fixture so the two halves cannot
/// drift apart — a change that made the definition an error again would fail the test
/// above, and one that dropped the warning fails this one.
#[test]
fn issue940_a_library_fn_a_stdlib_method_shadows_says_so() {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}c97_shadow_main.loft"), false);
    scopes::check(&mut p.data, &mut p.database);
    let msgs = p.diagnostics.lines().join("\n");
    assert!(
        msgs.contains("shadowed-by-method") && msgs.contains("`clamp`"),
        "a library `clamp` the stdlib methods on `float` must warn that its bare name is \
         taken; got: {msgs:?}"
    );
}

/// loft#940 — the boundary, read off RESULTS rather than off the diagnostic text.
///
/// A warning about where a call goes is only as good as the claim underneath it, so every
/// row here reads the value the call actually produced. The two SUBJECT rows show the
/// author's function losing its bare name; the two QUIET rows show the exemptions still
/// resolving to the library, which is what stops the lint from being "any shared name".
///
/// `private` is the row the filed report did not have: the reporter saw a CONSUMER call go
/// wrong, and the shadow is decided by the receiver type's method table, which the
/// library's OWN bare calls consult too. `issue940lib::floor_mod(7, 3)` is written to
/// answer 4 and answers the stdlib's 1 — so a library can be broken against itself, with
/// no consumer involved. Visibility is not the axis either: that function is not `pub`.
#[test]
fn issue940_the_shadow_boundary_holds_on_both_backends() {
    let s = sep_str();
    let main = format!("tests{s}lib{s}issue940_main.loft");
    let libs = format!("tests{s}lib");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            // This asserts on WARNINGS, which the parser produces.  The whole-program cache
            // is default-on and a warm bundle replays the diagnostics it recorded — correct,
            // but this test writes both source files fresh each run and then runs the same
            // program twice (once per backend), so the second invocation must re-derive the
            // warnings from THIS source rather than serve any earlier bundle.
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stdout.contains("bare=0 qualified=10 private=1 free=903 own=8"),
            "{backend}: the shadow boundary moved — bare/private must reach the STDLIB \
             (0, 1) while qualified reaches the library (10) and the two exempt shapes \
             stay reachable bare (903, 8); got stdout {stdout:?} stderr {stderr:?}"
        );
        // Exactly the two shadowed definitions speak. Counting matters: the lint fires at
        // the DECLARATION, so a duplicate would mean it runs once per parse pass, and a
        // third would mean an exemption stopped exempting.
        let warned = stderr.matches("shadowed-by-method").count();
        assert_eq!(
            warned, 2,
            "{backend}: expected the two shadowed definitions (`clamp`, `floor_mod`) to \
             warn once each and the two exempt ones (`sum_of`, `find`) to stay quiet; \
             got {warned} in {stderr:?}"
        );
        assert!(
            !stderr.contains("`sum_of`") && !stderr.contains("`find`"),
            "{backend}: an exempt shape warned — a stdlib FREE function of the same name \
             is outranked by the import, and a method on ANOTHER receiver type never \
             takes this call; got {stderr:?}"
        );
    }
}

/// loft#940 — `LOFT_NO_SHADOWED_BY_METHOD` silences it, and silences only it.
#[test]
fn issue940_the_lint_has_an_opt_out() {
    let s = sep_str();
    let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg("--lib")
        .arg(format!("tests{s}lib"))
        .arg(format!("tests{s}lib{s}issue940_main.loft"))
        .env("LOFT_ERRORS", "compact")
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_SHADOWED_BY_METHOD", "1")
        .output()
        .expect("failed to invoke the loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("shadowed-by-method"),
        "LOFT_NO_SHADOWED_BY_METHOD must silence the lint; got {stderr:?}"
    );
    assert!(
        stdout.contains("bare=0 qualified=10 private=1 free=903 own=8"),
        "the opt-out is a DIAGNOSTIC switch — resolution must be untouched; got {stdout:?}"
    );
}

/// @PLN13 C101 — `std`/`core` are reserved package names (a library may not claim a
/// language-namespace name), and `std::name` is the stdlib's qualified form — the escape
/// hatch that still reaches a stdlib symbol shadowed by a user def or a `use lib::*`.
/// @C101 — `std`/`core` are reserved; `std::name` is the stdlib's qualified form.
#[test]
fn pln13_c101_reserved_names_and_std_qualifier() {
    // The canonical reserved list refuses the namespace names, admits ordinary ones.
    assert!(loft::libscan::is_reserved_package_name("std"));
    assert!(loft::libscan::is_reserved_package_name("core"));
    assert!(!loft::libscan::is_reserved_package_name("regex"));
    assert!(!loft::libscan::is_reserved_package_name("stdlib"));

    // `std::name` resolves to the stdlib prelude: a program qualifying a stdlib call
    // parses clean (the escape hatch freezes with the contract).
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        "fn main() { x = std::max(3, 9); print(\"{x}\"); }",
        "c101_std.loft",
        false,
    );
    scopes::check(&mut p.data, &mut p.database);
    assert!(
        p.diagnostics.level() < Level::Error,
        "std::max must resolve to the stdlib; got: {:?}",
        p.diagnostics.lines()
    );
}

// ── loft#788: a bare name two packages both declare ───────────────────────────

/// Parse one `tests/lib` main file with the fixture libraries available.
fn parse_lib_main(file: &str) -> Parser {
    let s = sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{s}lib")];
    p.parse(&format!("tests{s}lib{s}{file}"), false);
    scopes::check(&mut p.data, &mut p.database);
    p
}

fn errors_of(p: &Parser) -> String {
    p.diagnostics.lines().join("\n")
}

/// loft#788 — a bare `Chunk` / `shared()` / `SHARED_C` that TWO packages declare
/// must be refused, naming both packages.
///
/// The pair of orders is the test, not either file alone: the defect was that
/// the same source line compiled to a different program depending on which
/// `use` came first, so a fix that refuses only one order has not fixed it. Both
/// files below are identical apart from the two swapped `use` lines.
#[test]
fn a_bare_name_two_packages_declare_is_refused_in_either_order() {
    for file in ["dupname_ab_main.loft", "dupname_ba_main.loft"] {
        let p = parse_lib_main(file);
        let msgs = errors_of(&p);
        assert!(
            p.diagnostics.level() >= Level::Error,
            "{file}: a bare ambiguous name must not silently pick a winner: {msgs}"
        );
        assert!(
            msgs.contains("`Chunk` is declared by more than one package"),
            "{file}: the message must name what is ambiguous: {msgs}"
        );
        // Both packages named, so the reader can pick — a message naming only
        // the winner would describe the bug rather than the choice.
        assert!(
            msgs.contains("dupname_a::Chunk") && msgs.contains("dupname_b::Chunk"),
            "{file}: both spellings must be offered: {msgs}"
        );
    }
}

/// loft#788 — a function and a constant collide the same way, and worse: both
/// import orders COMPILE and run, answering differently.
///
/// The struct case at least errors on a field; these two were silent.
#[test]
fn an_ambiguous_call_and_constant_are_refused_too() {
    let p = parse_lib_main("dupname_ab_main.loft");
    let msgs = errors_of(&p);
    assert!(
        msgs.contains("`shared` is declared by more than one package"),
        "a bare CALL is ambiguous the same way: {msgs}"
    );
    assert!(
        msgs.contains("`SHARED_C` is declared by more than one package"),
        "so is a bare constant: {msgs}"
    );
    // The storage spelling of a function is `n_shared`; a message telling
    // someone to write `dupname_a::n_shared` names something they cannot type.
    assert!(
        !msgs.contains("::n_"),
        "the mangled name must not reach the message: {msgs}"
    );
}

/// loft#788 control — QUALIFIED says which, so it must keep compiling.
///
/// Without this the fix would "pass" by refusing the collision everywhere,
/// including the shape #305 built to make work.
#[test]
fn a_qualified_name_is_never_ambiguous() {
    let p = parse_lib_main("dupname_qualified_main.loft");
    assert!(
        p.diagnostics.level() < Level::Error,
        "qualified names say which package they mean: {}",
        errors_of(&p)
    );
}

/// loft#788 control — two packages may share a name a program never writes
/// bare, and that program compiles today.
///
/// This is why the refusal is at the USE and not at the `use`: reporting when
/// the import is applied would break a working program for a collision it never
/// has (COMPATIBILITY.md — no functioning program breaks).
#[test]
fn an_unused_collision_still_compiles() {
    let p = parse_lib_main("dupname_unused_main.loft");
    assert!(
        p.diagnostics.level() < Level::Error,
        "an unused collision is not a question: {}",
        errors_of(&p)
    );
}

// ── loft#850: a method and a free function of one name, across packages ───────

/// loft#850 — a bare call reaches the function that accepts the receiver it was
/// GIVEN, whichever package declared it.
///
/// Three packages here each declare a `struct Thing`, and a method is filed under
/// the mangled key `t_5Thing_go`, which spells the type's NAME and nothing about
/// its package — so all three competed for one key and the first import won it.
/// The runtime half (which package's body actually ran) is pinned across all
/// three backends by `tests/scripts/850-cross-package-method-name-collision.loft`
/// and its swapped-order twin; this asserts the compile-time half, that nothing
/// is refused.
#[test]
fn a_method_and_a_free_function_of_one_name_resolve_by_receiver() {
    for file in ["dupmethod_ab_main.loft", "dupmethod_ba_main.loft"] {
        let p = parse_lib_main(file);
        assert!(
            p.diagnostics.level() < Level::Error,
            "{file}: the receiver says which `go` is meant: {}",
            errors_of(&p)
        );
    }
}

/// loft#850 — when the call genuinely cannot resolve, the hint names the
/// receiver in a form the reader can type, and does not blame the stdlib for a
/// package's choice.
///
/// The bare receiver name is the one spelling that identifies nothing in this
/// situation, since several packages declare it; and "stdlib declared `go` as a
/// method" pointed at a file that never mentions `go`. Both halves are asserted
/// because fixing either alone still leaves the reader without a next step.
#[test]
fn an_unresolvable_call_names_the_package_that_declares_the_method() {
    let p = parse_lib_main("dupmethod_hint_main.loft");
    let msgs = errors_of(&p);
    assert!(
        msgs.contains("dupmethod_b::Thing"),
        "the hint must name the receiver's package: {msgs}"
    );
    assert!(
        !msgs.contains("stdlib declared"),
        "the stdlib declared nothing here: {msgs}"
    );
}

/// @PLN102 C98 / loft#852 — a library's public function must not claim the
/// CONSUMER's variable namespace.
///
/// `use lib;` beside a local named after one of the library's `pub fn`s has to
/// compile, or every short verb a library exports (`turn`, `step`, `run`,
/// `wait`, `next`, `open`, `send`) becomes a word no consumer of that library
/// may use as a local — a break that arrives on someone else's release, that
/// nothing announces, and that no consumer can prepare for.
///
/// Values and functions live in separate namespaces, so all three facts hold at
/// once and all three are asserted: the local keeps its own value, an
/// unqualified call reaches the library function from a scope with no such
/// local, and a call reaches it from the scope that has one. Asserting only
/// "it compiles" would pass a fix that bound the name to the wrong thing.
///
/// Run on BOTH backends through the binary rather than parsed in-process: the
/// resolution is the parser's, but the values are what a consumer sees, and
/// only running proves the call did not silently answer the local.
/// @C112 — a local may shadow a library function.
#[test]
fn pln102_c98_a_local_may_shadow_a_library_function() {
    let s = sep_str();
    let main = format!("tests{s}lib{s}issue852_main.loft");
    let libs = format!("tests{s}lib");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("turn=42 call=200 other=300"),
            "{backend}: a local named after a library's `pub fn` must bind (42), \
             and the function stay callable from both a scope with the local (200) \
             and one without (300); got stdout {stdout:?} stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// loft#853 — a library's free function must win its own QUALIFIED call, even when the
/// stdlib declares that name as a METHOD on the first argument's type.
///
/// loft#850 taught the method lookup to re-ask in the receiver type's OWN source when
/// the caller's scope answered with another package's method. But every type has an own
/// source, and for a builtin it is the stdlib — so searching it unconditionally let a
/// stdlib method on `text` outrank a library's free function of the same name.
/// `regex::split(pattern, input)` resolved to `split(self: text, separator: character)`
/// and the published `regex` package stopped compiling, which the freeze forbids.
///
/// The control line matters as much as the subject: a fix that reached the free function
/// by losing the stdlib's own text surface would satisfy the first assertion and break
/// every program in the language. Run through the binary so the resolution is checked by
/// its RESULT, not by the absence of a diagnostic — a wrong-but-compiling resolution is
/// exactly what loft#850 was about.
#[test]
fn issue853_a_library_free_fn_outranks_a_stdlib_method_of_the_same_name() {
    let s = sep_str();
    let main = format!("tests{s}lib{s}issue853_main.loft");
    let libs = format!("tests{s}lib");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("qualified=5 stdlib_free=3 stdlib_method=true"),
            "{backend}: `lib::split(text, text)` must reach the LIBRARY's free fn (5) while \
             the stdlib's own free fn (3) and text methods (true) still resolve; \
             got stdout {stdout:?} stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// ── loft#1080: one FILE, two names, two parses ────────────────────────────────

/// loft#1080 — a module reachable under two names must be loaded ONCE.
///
/// A package's own module is keyed `<pkg>::<module>` so that no other package can take
/// the name from under it, and that is exactly what lets one file arrive at the loader
/// twice: a program OUTSIDE the package `use`s the module by its bare name, then a file
/// INSIDE the package `use`s the same module and computes the qualified key, which is
/// absent — so the same file is parsed into a SECOND source.
///
/// Nothing downstream survives that. The bare name is then rebound to the second source,
/// leaving the first source's definitions unreachable but still present, and both
/// symptoms follow from that one cause:
///
/// * every bare call becomes AMBIGUOUS, and the message names a module the author never
///   wrote — *"`grid_value` is declared by more than one module here —
///   `issue1080_pkg::grid1080::grid_value` and `src2::grid_value`"*, `src2` being the
///   orphaned second source. That is what this fixture hits, on both backends.
/// * where the program is big enough to reach native codegen, every duplicated function
///   is emitted twice under ONE identifier: `disambiguated_fn_ident` separates same-named
///   functions by hashing their defining FILE, on the stated ground that two of them can
///   only come from different files — so a second parse of one file defeats it and the
///   generated cdylib will not compile. The reporting project got 55 ×
///   `error[E0428] … loft_shared_n_…_mafed3b7f is defined multiple times`, each pair
///   carrying an IDENTICAL hash, which is the tell that it is one file twice rather than
///   loft#305's two different files.
///
/// The issue was filed as "the same library reachable at two paths", and it is not: it
/// reproduces from a SINGLE `--lib`, and the reporter's second copy on disk was a
/// coincidence of the invocation. What decides it is the package manifest above the lib
/// directory, which is what gives the inside-the-package `use` its qualified key.
///
/// Both values are asserted, not just that it compiles: deduplicating by dropping one of
/// the two loads would also make the ambiguity go away, and only the answers say the
/// surviving load is the right one.
#[test]
fn issue1080_a_module_reached_by_two_names_is_loaded_once() {
    let s = sep_str();
    let main = format!("tests{s}lib{s}issue1080_main.loft");
    let libs = format!("tests{s}lib{s}issue1080_pkg{s}src");
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg(backend)
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("flat=7 inside=8"),
            "{backend}: the bare `use grid1080` from outside the package and the \
             `issue1080_pkg::grid1080` one from inside it name ONE file, so both routes \
             must reach the one `grid_value` (7) and the module built on it (8); \
             got stdout {stdout:?} stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// loft#1080 control — two DIFFERENT files that happen to share a module name must still
/// be told apart.
///
/// The fix keys the "already loaded" test on the CANONICAL PATH, so it collapses one file
/// reached twice and nothing else. Were it keyed on the module's short name instead, this
/// pair would silently become one module — the failure loft#912 and loft#949 exist to
/// report, arrived at from the opposite direction.
///
/// `dupname_a` and `dupname_b` each declare a bare `Chunk` / `shared()` / `SHARED_C`, and
/// the refusal that names both packages is asserted by
/// `a_bare_name_two_packages_declare_is_refused_in_either_order` above. Re-running it here
/// is the point: this test fails if the dedup ever widens from "the same file" to "the
/// same name".
#[test]
fn issue1080_two_different_files_of_one_name_are_still_distinct() {
    for file in ["dupname_ab_main.loft", "dupname_ba_main.loft"] {
        let p = parse_lib_main(file);
        let msgs = errors_of(&p);
        assert!(
            msgs.contains("dupname_a::Chunk") && msgs.contains("dupname_b::Chunk"),
            "{file}: two distinct files must stay two modules, however alike their names: \
             {msgs}"
        );
    }
}

/// loft#1094 — the duplicate-name check is scoped to a SOURCE's namespace, and a
/// type-mismatch between two same-named types has to say which is which.
///
/// The issue reported the check as unreliable: it fires in a minimal package and not in a
/// real one "with the same ingredients". The ingredients differ in exactly one axis, and
/// it is the IMPORT FORM. A wildcard `use dep::*;` puts every public name into this
/// source's namespace, so declaring one of them again is a genuine redefinition and is
/// refused; a SELECTIVE `use dep::(a, b);` imports only what it names, and a bare
/// `use dep;` imports no name at all — only the `dep::` qualifier (@C98) — so a local
/// declaration of any other name is an ordinary, unambiguous definition, which is the
/// coexistence @PLN102 C97 blessed.
///
/// The first three cells pin that axis so no regime can drift into another. The
/// third is the defect the investigation did turn up: when two live types share a name,
/// `expected Frame, got Frame` named them identically and left the reader nothing to go
/// on. Naming both DECLARATION sites is the one case where the position of a type, rather
/// than of the offending value, is the useful fact.
#[test]
fn issue1094_import_form_decides_a_name_clash_and_a_clash_names_both_sites() {
    let dir = std::env::temp_dir().join("loft_issue1094");
    let _ = std::fs::remove_dir_all(&dir);
    let libs = dir.join("libs");
    std::fs::create_dir_all(&libs).expect("libs");
    std::fs::write(
        libs.join("depa.loft"),
        "pub struct Frame { fa_x: float }\npub fn depa_helper() -> integer { 1 }\n",
    )
    .expect("depa");
    std::fs::write(
        libs.join("depb.loft"),
        "pub struct Frame { fb_y: float }\npub fn takes_frame(f: Frame) -> float { f.fb_y }\n",
    )
    .expect("depb");

    let run = |name: &str, body: &str| -> String {
        let main = dir.join(name);
        std::fs::write(&main, body).expect("main");
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg("--interpret")
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    // A — the wildcard puts `Frame` in this source, so declaring it again is refused.
    let wildcard = run(
        "wildcard.loft",
        "use depa::*;\nstruct Frame { fm_x: float }\nfn main() { print(\"{depa_helper()}\") }\n",
    );
    assert!(
        wildcard.contains("conflicts with"),
        "`use depa::*;` imports `Frame`, so a local one is a redefinition: {wildcard}"
    );

    // A2 — the bare import brings in only the `depa::` qualifier (@C98), so the local
    // `Frame` is ordinary and the library is reached through its qualifier.
    let bare = run(
        "bare.loft",
        "use depa;\nstruct Frame { fm_x: float }\n\
         fn main() { f = Frame { fm_x: 6.0 }; print(\"local={f.fm_x} dep={depa::depa_helper()}\") }\n",
    );
    assert!(
        bare.contains("local=6 dep=1"),
        "a bare `use depa;` leaves every name free, so the LOCAL `Frame` is used: {bare}"
    );

    // B — the selective import does not, so the local `Frame` is ordinary and is USED.
    let selective = run(
        "selective.loft",
        "use depa::(depa_helper);\nstruct Frame { fm_x: float }\n\
         fn main() { f = Frame { fm_x: 4.0 }; print(\"local={f.fm_x} dep={depa_helper()}\") }\n",
    );
    assert!(
        !selective.contains("conflicts with"),
        "a selective import leaves the name free — refusing here would reject the \
         coexistence C97 blessed: {selective}"
    );
    assert!(
        selective.contains("local=4 dep=1"),
        "and the LOCAL `Frame` is the one used: {selective}"
    );

    // C — two live `Frame`s meeting at a call must be told apart by their declarations.
    let clash = run(
        "clash.loft",
        // `Frame` here is depa's (wildcard import); `takes_frame` is imported by NAME
        // only, so depb's `Frame` never enters this source — the two types meet at the call.
        "use depa::*;\nuse depb::(takes_frame);\n\
         fn main() { print(\"{takes_frame(Frame { fa_x: 1.0 })}\") }\n",
    );
    assert!(
        clash.contains("two different types share this name"),
        "a mismatch between two same-named types must say so: {clash}"
    );
    assert!(
        clash.contains("depa.loft") && clash.contains("depb.loft"),
        "and must name BOTH declaration sites — naming neither is what made the old \
         `expected Frame, got Frame` unreadable: {clash}"
    );
}

/// loft#1147 — a LIBRARY's bounded generic must not swallow the consumer's own struct.
///
/// A type variable's bound stubs are keyed by its NAME (`t_1T_to_text`), and that is the same
/// string a user `struct T` mangles to.  A library declaring `fn render<T: Printable>(v: T)`
/// therefore minted a stub that the consumer's `"{T { … }}"` interpolation resolved by name —
/// and monomorphisation could not resolve it for a struct that is not the type variable, so
/// every such value rendered as EMPTY, on both backends, with no diagnostic.
///
/// This is the RUNTIME half, so it runs the binary rather than reading diagnostics: the
/// defect renders `[]` where `[{z:9}]` is correct, and neither is an error.  The library call
/// beside it is the control — it must keep working, since the fix distinguishes the stub's
/// owner rather than disabling the stub.
#[test]
fn a_library_s_bounded_generic_does_not_swallow_a_same_named_struct() {
    let dir = std::env::temp_dir().join("loft_tv_bound_collision");
    let libdir = dir.join("lib");
    std::fs::create_dir_all(&libdir).expect("create temp lib dir");
    std::fs::write(
        libdir.join("tvboundlib.loft"),
        "pub fn render<T: Printable>(v: T) -> text { \"<{v}>\" }\n",
    )
    .expect("write lib");
    let main = dir.join("main.loft");
    std::fs::write(
        &main,
        "use tvboundlib::*;\nstruct T { z: integer }\nfn main() { println(\"[{T{z:9}}] {render(4)}\"); }\n",
    )
    .expect("write main");
    let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg("--lib")
        .arg(libdir.as_os_str())
        .arg(main.as_os_str())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        stdout.contains("[{z:9}]"),
        "the consumer's own `struct T` must render its fields, not empty; got stdout {stdout:?} stderr {stderr:?}"
    );
    assert!(
        stdout.contains("<4>"),
        "CONTROL: the library's bounded generic must keep working; got stdout {stdout:?}"
    );
}

/// loft#1153 — TWO libraries' bounded generics, and a consumer that declares the name.
///
/// This is the shape a registry produces and the one no single-repo test reaches: a generic's
/// type variable and a struct of the same name shared ONE method namespace, so a bound
/// declaring a method as common as `to_text` or `op ==` reserved it against every struct
/// spelling the variable's name. Measured on the pre-fix build, this program earns BOTH
/// `Cannot redefine 'OpEq' (already defined at lib/libb.loft)` and `Cannot redefine 'to_text'
/// (already defined at lib/liba.loft)` — and **neither library author can observe that**, which
/// is the argument for fixing rather than documenting: a documented landmine only works if the
/// person who steps on it can read the sign, and here the sign would have to be in two other
/// people's READMEs.
///
/// Every resolution is asserted, not just the absence of an error: the consumer's own
/// `to_text` and `OpEq` must win for its own type, and both libraries' generics must still
/// work — a fix that silenced the redefinition by dropping a stub would pass an
/// error-count-only check.
#[test]
fn two_libraries_bounded_generics_leave_a_consumer_s_own_type_alone() {
    let dir = std::env::temp_dir().join("loft_two_lib_holder_namespace");
    let libdir = dir.join("lib");
    std::fs::create_dir_all(&libdir).expect("create temp lib dir");
    std::fs::write(
        libdir.join("holdera.loft"),
        "pub fn show_a<T: Printable>(v: T) -> text { \"A<{v}>\" }\n",
    )
    .expect("write lib a");
    std::fs::write(
        libdir.join("holderb.loft"),
        "pub fn show_b<T: Equatable>(a: T, b: T) -> boolean { a != b }\n",
    )
    .expect("write lib b");
    let main = dir.join("main.loft");
    std::fs::write(
        &main,
        "use holdera::*;\n\
         use holderb::*;\n\
         struct T { z: integer }\n\
         fn OpEq(self: T, other: T) -> boolean { self.z == other.z }\n\
         operator to_text(self: T) -> text { \"T<{self.z}>\" }\n\
         fn main() {\n\
         t1 = T { z: 1 };\n\
         t2 = T { z: 2 };\n\
         println(\"[{t1}] {show_a(4)} {show_b(1,2)} {t1 == t2} {t1.to_text()}\");\n\
         }\n",
    )
    .expect("write main");
    let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg("--lib")
        .arg(libdir.as_os_str())
        .arg(main.as_os_str())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        stdout.contains("[T<1>] A<4> true false T<1>"),
        "the consumer's own `to_text` and `OpEq` must win for its own type, and BOTH libraries' \
         generics must still resolve; got stdout {stdout:?} stderr {stderr:?}"
    );
}

/// @C98 — the error for a name a facade package uses without passing it on names the RIGHT
/// package, and prescribes only a cure that compiles.
///
/// Two packages each ship a module called `inner`; `fac`'s holds the struct.  With the other
/// package loaded, the short name `inner` answers for whichever package bound it last, so:
/// - the error blamed `fac2` for a name only `fac` uses (the bare `use` was looked up again by
///   name, and found `fac2`'s module);
/// - it prescribed `inner::Thing` where `inner` names the other module, and a
///   `fac::inner::Thing` spelling does not parse at all.
///
/// Now the facade is the one whose `use` bound the module, `inner::Thing` is offered only
/// where it resolves to that module, and each cure the message names is written and run.
#[test]
fn a_name_a_facade_keeps_names_the_facade_and_a_cure_that_compiles() {
    let tmp = std::env::temp_dir().join(format!("loft_c98_fac_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let pkg = |name: &str, entry: &str, inner: &str| {
        let d = tmp.join(name);
        std::fs::create_dir_all(d.join("src")).expect("package dir");
        std::fs::write(
            d.join("loft.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n\
                 [library]\nentry = \"src/{name}.loft\"\n"
            ),
        )
        .expect("manifest");
        std::fs::write(d.join(format!("src/{name}.loft")), entry).expect("entry");
        std::fs::write(d.join("src/inner.loft"), inner).expect("module");
        d
    };
    let fac = pkg(
        "fac",
        "use inner;\npub fn mk() -> inner::Thing { inner::Thing { a: 3 } }\n",
        "pub struct Thing { a: integer }\n",
    );
    let fac2 = pkg(
        "fac2",
        "use inner;\npub fn mk2() -> integer { inner::other() }\n",
        "pub fn other() -> integer { 9 }\n",
    );
    let facp = pkg(
        "facp",
        "use inner;\npub use inner::(Thing);\npub fn mk() -> inner::Thing { inner::Thing { a: 3 } }\n",
        "pub struct Thing { a: integer }\n",
    );
    let run = |libs: &[&std::path::Path], program: &str| {
        let src = tmp.join("prog.loft");
        std::fs::write(&src, program).expect("program");
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_loft"));
        c.arg("--interpret").arg(&src);
        for l in libs {
            c.arg("--lib").arg(l);
        }
        let out = c
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "120")
            .output()
            .expect("run loft");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let bare = "fn main() { t: Thing = mk(); println(\"{t.a}\"); }\n";
    let qualified = "fn main() { t: inner::Thing = mk(); println(\"{t.a}\"); }\n";

    // `fac` loaded after `fac2`: `inner` names fac's module, so the qualifier is a cure.
    let (_, err) = run(
        &[&fac, &fac2],
        &format!("use fac2::*;\nuse fac::*;\n{bare}"),
    );
    assert!(
        err.contains("`fac` uses it from `inner`") && err.contains("write `inner::Thing`"),
        "names fac and the qualifier:\n{err}"
    );
    let (out, err) = run(
        &[&fac, &fac2],
        &format!("use fac2::*;\nuse fac::*;\n{qualified}"),
    );
    assert_eq!(out, "3\n", "the qualifier it names compiles:\n{err}");

    // `fac2` loaded after `fac`: `inner` names fac2's module, so only the facade can cure it.
    let (_, err) = run(
        &[&fac, &fac2],
        &format!("use fac::*;\nuse fac2::*;\n{bare}"),
    );
    assert!(
        err.contains("`fac` uses it from its module `inner`")
            && err.contains("has no name this file can write")
            && !err.contains("write `inner::Thing`"),
        "names fac, and offers no qualifier that resolves elsewhere:\n{err}"
    );
    let (_, err) = run(
        &[&fac, &fac2],
        &format!("use fac::*;\nuse fac2::*;\n{qualified}"),
    );
    assert!(
        err.contains("Undefined type inner::Thing"),
        "that qualifier would not compile:\n{err}"
    );

    // The facade's cure, as the message spells it, compiles in the same order.
    let (out, err) = run(
        &[&facp, &fac2],
        &format!("use facp::*;\nuse fac2::*;\n{bare}"),
    );
    assert_eq!(out, "3\n", "`pub use inner::(Thing);` passes it on:\n{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A type named by a construction AND by a written annotation, and resolved by neither, is
/// reported once with the import cure (@C98) — never an internal compiler error.
///
/// `use lat;` binds only the `lat::` qualifier, so `Hex` is unresolved in this file.  The
/// construction `nb(Hex { … }, 1)` registers a speculative stub for it; the annotation
/// `d: vector<Hex>` then names the same stub.  Left speculative, nothing reported the
/// annotation, and `for h in d` read `def(u32::MAX)` (dryopea's `damage.loft` under C98).
/// A misspelled name used both ways is the single-file form:
/// `issues::a_type_both_constructed_and_annotated_is_reported_once`.
#[test]
fn an_unimported_type_both_constructed_and_annotated_names_the_import_cure() {
    let dir = std::env::temp_dir().join(format!("loft_c98_stub_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let libs = dir.join("libs");
    std::fs::create_dir_all(&libs).expect("libs");
    std::fs::write(
        libs.join("lat.loft"),
        "pub struct Hex { q: integer, r: integer }\n\
         pub fn nb(h: Hex, d: integer) -> Hex { Hex { q: h.q + d, r: h.r } }\n",
    )
    .expect("lat");
    let run = |name: &str, body: &str| -> (bool, String) {
        let main = dir.join(name);
        std::fs::write(&main, body).expect("main");
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg("--interpret")
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    };
    let body = |hex: &str| {
        format!(
            "use lat;\n\
             fn a(q: integer) -> integer {{\n  x = lat::nb({hex} {{ q: q, r: 0 }}, 1);\n  x.q\n}}\n\
             fn g() -> integer {{\n  d: vector<{hex}> = [];\n  for i in 0..3 {{ d += [{hex} {{ q: i, r: 0 }}]; }}\n  \
             n = 0;\n  for h in d {{ n += h.q; }}\n  n\n}}\n\
             fn main() {{ print(\"{{g()}} {{a(5)}}\") }}\n"
        )
    };
    let (ok, bare) = run("bare.loft", &body("Hex"));
    assert!(
        !ok,
        "`Hex` is not imported, so the program is refused:\n{bare}"
    );
    assert!(
        !bare.contains("internal compiler error"),
        "an unresolved type is a user error, not a compiler fault:\n{bare}"
    );
    assert!(
        bare.contains("Undefined type Hex") && bare.contains("use lat::(Hex);"),
        "the report names the import cure:\n{bare}"
    );
    assert_eq!(
        bare.lines().filter(|l| l.starts_with("Error:")).count(),
        1,
        "one cause, one error — no `Unknown variable 'd'` beside it:\n{bare}"
    );
    // The cure the error names compiles and answers the hand-computed values:
    // g() = 0 + 1 + 2, a(5) = 5 + 1.
    let (ok, cured) = run("cured.loft", &body("lat::Hex"));
    assert!(
        ok && cured.contains("3 6"),
        "the qualified spelling is the cure:\n{cured}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A CONSTANT behind a bare `use` gets the same cure as a function or a type (@C98): the
/// report names the library and both spellings, not a bare "Unknown variable".  dryopea's
/// `CLOCK_UNITS_PER_SECOND` after `use fixstep;` is the shape.
#[test]
fn a_constant_behind_a_bare_use_names_the_import_cure() {
    let dir = std::env::temp_dir().join(format!("loft_c98_const_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let libs = dir.join("libs");
    std::fs::create_dir_all(&libs).expect("libs");
    std::fs::write(libs.join("clk.loft"), "pub const UNITS: integer = 3000;\n").expect("clk");
    let run = |name: &str, body: &str| -> (bool, String) {
        let main = dir.join(name);
        std::fs::write(&main, body).expect("main");
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg("--interpret")
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    };
    let (ok, bare) = run(
        "bare.loft",
        "use clk;\nfn main() { print(\"{UNITS / 1000}\") }\n",
    );
    assert!(!ok, "`UNITS` is not imported:\n{bare}");
    assert!(
        bare.contains("it is in `clk`") && bare.contains("`clk::UNITS`"),
        "the report names the library and the qualified spelling:\n{bare}"
    );
    let (ok, cured) = run(
        "cured.loft",
        "use clk;\nfn main() { print(\"{clk::UNITS / 1000}\") }\n",
    );
    assert!(
        ok && cured.contains('3'),
        "the qualified spelling is the cure:\n{cured}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// loft#1769 — a name TWO plain imports deep names the loaded library that has it.  `main`
/// does `use top::*;`, `top` does `use mid::*;`, and `mid` does `use deep::*;` for its own
/// use: nothing passes `deep`'s names on, and each import-form cure looks only one step down,
/// so the report was a bare "Undefined type Deep" (dryopea met it as "Undefined type Mesh —
/// did you mean 'hash'?" through `graphics`'s own `use mesh3d::*;`).  Each kind of name gets
/// the cure, each cure it names is compiled, and a name two loaded libraries both declare
/// gets no cure, since naming one of them would be a guess.
#[test]
fn a_name_two_imports_deep_names_the_library_that_has_it() {
    let dir = std::env::temp_dir().join(format!("loft_1769_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let libs = dir.join("libs");
    std::fs::create_dir_all(&libs).expect("libs");
    for (name, body) in [
        ("top", "use mid::*;\npub fn t() -> integer { m() + 1 }\n"),
        (
            "mid",
            "use deep::*;\nuse twin;\npub fn m() -> integer { make_deep(4).d }\n",
        ),
        (
            "deep",
            "pub struct Deep { d: integer }\npub struct Pair { p: integer }\n\
             pub fn make_deep(n: integer) -> Deep { Deep { d: n * 10 } }\n\
             pub const DEPTH: integer = 9;\n",
        ),
        ("twin", "pub struct Pair { q: integer }\n"),
    ] {
        std::fs::write(libs.join(format!("{name}.loft")), body).expect("lib");
    }
    let run = |name: &str, body: &str| -> (bool, String) {
        let main = dir.join(name);
        std::fs::write(&main, body).expect("main");
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg("--interpret")
            .arg("--lib")
            .arg(&libs)
            .arg(&main)
            .env("LOFT_ERRORS", "compact")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("failed to invoke the loft binary");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    };
    for (tag, body, head) in [
        ("type", "x: Deep = t(); ", "Undefined type Deep"),
        (
            "function",
            "x = make_deep(t()); ",
            "Unknown function make_deep",
        ),
        ("constant", "x = DEPTH + t(); ", "Unknown variable"),
    ] {
        let (ok, text) = run(
            &format!("{tag}.loft"),
            &format!("use top::*;\nfn main() {{ {body}print(\"x\") }}\n"),
        );
        assert!(!ok, "{tag}: not imported:\n{text}");
        assert!(
            text.contains(head)
                && text.contains("it is in `deep`, which nothing this file imports"),
            "{tag}: the report names the library that has it:\n{text}"
        );
    }
    // The three cures the report names, each compiled and run: t() = 4 * 10 + 1.
    for (tag, imports, expr) in [
        (
            "by_name",
            "use deep::(make_deep, DEPTH);",
            "make_deep(t()).d + DEPTH",
        ),
        ("glob", "use deep::*;", "make_deep(t()).d + DEPTH"),
        (
            "qualified",
            "use deep;",
            "deep::make_deep(t()).d + deep::DEPTH",
        ),
    ] {
        let (ok, text) = run(
            &format!("cure_{tag}.loft"),
            &format!("use top::*;\n{imports}\nfn main() {{ print(\"{{{expr}}}\") }}\n"),
        );
        assert!(
            ok && text.contains("419"),
            "{tag}: the cure compiles:\n{text}"
        );
    }
    // `Pair` is declared by `deep` AND `twin`: no single library is the answer.
    let (ok, text) = run(
        "twin.loft",
        "use top::*;\nfn main() { x: Pair = t(); print(\"x\") }\n",
    );
    assert!(
        !ok && text.contains("Undefined type Pair") && !text.contains("it is in `"),
        "two libraries declare it, so no cure is guessed:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A library directory's HIDDEN entries are no part of it (loft#1761): `.loft/` is the cache a
/// run writes beside its sources, `.git/` a checkout's history.  A `.loft` file under one is
/// not loaded, and its definitions do not exist; a visible subdirectory still is.
#[test]
fn a_hidden_directory_in_a_library_is_not_loaded() {
    let dir = std::env::temp_dir().join(format!("loft_hidden_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".loft").join("cache")).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.loft"), "pub fn hd_visible() -> integer { 1 }\n").unwrap();
    std::fs::write(
        dir.join("sub").join("b.loft"),
        "pub fn hd_nested() -> integer { 2 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".loft").join("cache").join("c.loft"),
        "pub fn hd_hidden() -> integer { 3 }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_dir(&dir.to_string_lossy(), false, false).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_ne!(
        p.data.def_nr("n_hd_visible"),
        u32::MAX,
        "a file in the directory loads"
    );
    assert_ne!(
        p.data.def_nr("n_hd_nested"),
        u32::MAX,
        "a visible subdirectory loads"
    );
    assert_eq!(
        p.data.def_nr("n_hd_hidden"),
        u32::MAX,
        "a file under a hidden directory must not load"
    );
}
