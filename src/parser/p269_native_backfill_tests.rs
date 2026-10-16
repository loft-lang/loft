// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::Parser;

/// P269 regression — with MORE THAN ONE native package registered,
/// `backfill_native_symbol_crates` must still bind each unmapped `#native`
/// symbol to its owning package (the registered package whose directory is a
/// prefix of the def's source file).  The old `len() != 1` guard bailed and
/// left them unmapped → a `--native` P269 compile error on a symbol the
/// interpreter dispatched fine.  Two native packages (`imaging` +
/// `p269_native_b`); we then clear the map to reproduce the crawler
/// precondition (imaging's `#native` symbols left unbound by the per-manifest
/// pass, which runs before the package source is parsed) so backfill is their
/// only binding site.
#[test]
fn backfill_binds_symbols_under_multiple_native_packages() {
    let sep = crate::platform::sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![
        format!("tests{sep}lib"),
        format!("tests{sep}fixtures{sep}libs"),
    ];
    p.parse(
        &format!("tests{sep}lib{sep}p269_two_native_pkgs_main.loft"),
        false,
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "unexpected parse errors: {:?}",
        p.diagnostics.lines()
    );

    // The bug trigger: more than one registered native package.
    assert!(
        p.data.native_packages.len() >= 2,
        "expected >= 2 native packages (imaging + p269_native_b), got {:?}",
        p.data.native_packages
    );

    // Collect imaging's #native symbols FROM the parsed defs (those whose
    // source lives under imaging's package dir), so the test never hardcodes
    // a library `n_*` symbol in src/ — the @PLAN12 extraction-hygiene gate
    // forbids that, since imaging is an extracted library.
    let imaging_dir = p
        .data
        .native_packages
        .iter()
        .find(|(c, _)| c == "loft-imaging")
        .map(|(_, d)| d.clone())
        .expect("imaging registered as a native package");
    let imaging_syms: Vec<String> = (0..p.data.definitions())
        .map(|d| p.data.def(d))
        .filter(|def| {
            !def.native().is_empty()
                && crate::file_access::is_under(def.position().file.as_str(), &imaging_dir)
        })
        .map(|def| def.native().to_string())
        .collect();
    assert!(
        !imaging_syms.is_empty(),
        "expected imaging to contribute #native symbols"
    );

    // Reproduce the precondition the crawler hit: those symbols are not yet
    // bound, so backfill is the only place they can bind.
    p.data.native_symbol_crates.clear();
    p.backfill_native_symbol_crates();

    for sym in &imaging_syms {
        assert_eq!(
            p.data.native_symbol_crates.get(sym).map(String::as_str),
            Some("loft_imaging"),
            "{sym} not bound to loft_imaging after backfill - map={:?}",
            p.data.native_symbol_crates
        );
    }
}

/// @PLN40 step 3 — `const` on a struct field sets `const_field` on that
/// attribute and leaves plain fields untouched.  Non-vacuous: a no-op mark
/// fails the first assert, an over-eager mark the second.
#[test]
fn pln40_const_field_flag_is_set() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(
        "struct Cell { const c_color: integer, height: integer }",
        "pln40_const_field_flag_is_set",
        false,
    );
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "unexpected parse errors: {:?}",
        p.diagnostics.lines()
    );
    let cell = p.data.def_nr("Cell");
    assert_ne!(cell, u32::MAX, "Cell def not found");
    let color = p.data.attr(cell, "c_color");
    let height = p.data.attr(cell, "height");
    assert_ne!(color, usize::MAX, "c_color attr missing");
    assert_ne!(height, usize::MAX, "height attr missing");
    assert!(
        p.data.def(cell).attributes()[color].const_field,
        "const field c_color must set const_field"
    );
    assert!(
        !p.data.def(cell).attributes()[height].const_field,
        "plain field height must NOT be const"
    );
}

/// @PLN26 phase 1 — two native packages exporting the SAME `#native` symbol
/// must be detected: the C-ABI flat namespace (and the interpreter's
/// symbol-keyed `BRIDGE_REGISTRY`) can't disambiguate them, so native
/// codegen rejects the program with a `compile_error!`.  Parse a consumer of
/// collide_a + collide_b (both export `collide_shared`) and check the
/// detector reports it.
#[test]
fn native_symbol_collision_across_packages_detected() {
    let sep = crate::platform::sep_str();
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.lib_dirs = vec![format!("tests{sep}lib")];
    p.parse(&format!("tests{sep}lib{sep}collide_main.loft"), false);
    let collisions = p.data.native_symbol_collisions();
    assert!(
        collisions
            .iter()
            .any(|(sym, srcs)| sym == "collide_shared" && srcs.len() >= 2),
        "expected a `collide_shared` collision across >= 2 sources, got {collisions:?}"
    );
}
