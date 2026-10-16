// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

use super::Parser;
use crate::sandbox::parse_sandbox_config;

fn parse_with_sandbox(selectors: &[&str], src: &str) -> Parser {
    let list = selectors
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let cfg = format!("[sandbox]\nmod = [{list}]\n[profile.mod]\nallow = [\"x#read\"]\n");
    let mut p = Parser::new();
    p.set_sandbox_config(parse_sandbox_config(&cfg));
    p.parse_dir("default", true, true).unwrap(); // `integer` et al. live in the stdlib
    // Process-global counter (not the `src` pointer) for a collision-free name
    // across concurrent test threads — see `parse_admit_libs`.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "plan86_reach_{}_{}.loft",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    crate::file_access::write(&path, src).unwrap();
    p.parse(path.to_str().unwrap(), false);
    let _ = crate::file_access::remove_file(&path);
    p
}

/// @PLN86 1.3 — the closure descends into sandboxed defs (so their callees are
/// reachable) but treats a trusted symbol as a LEAF: `untrusted_helper` is
/// recorded yet NOT descended, so the `hidden_leaf` it alone reaches is
/// excluded — the trust boundary §4 demands.
#[test]
fn closure_descends_sandboxed_defs_and_stops_at_trusted_leaves() {
    let src = "fn allowed_leaf() -> integer { 1 }\n\
               fn hidden_leaf() -> integer { 2 }\n\
               fn inner() -> integer { allowed_leaf() }\n\
               fn untrusted_helper() -> integer { hidden_leaf() }\n\
               fn scripted() -> integer { inner(); untrusted_helper() }\n";
    let p = parse_with_sandbox(&["fn:scripted", "fn:inner"], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    let reach = p.sandbox_reachable_set();
    let id = |n: &str| p.data.def_nr(n);
    assert!(reach.contains(&id("n_scripted")));
    assert!(reach.contains(&id("n_inner"))); // sandboxed → descended
    assert!(reach.contains(&id("n_allowed_leaf"))); // reached via inner
    assert!(reach.contains(&id("n_untrusted_helper"))); // referenced → leaf
    assert!(
        !reach.contains(&id("n_hidden_leaf")),
        "a trusted leaf must not be descended"
    );
}

/// L4 — a fn-ref passed as a `fn(...)`-typed argument (emitted as a bare
/// `Int(def_nr)`) is in the set, so an indirect call can't escape admission.
#[test]
fn fnref_passed_as_argument_is_reachable_l4() {
    let src = "fn target(n: integer) -> integer { n }\n\
               fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
               fn scripted() -> integer { apply(target, 5) }\n";
    let p = parse_with_sandbox(&["fn:scripted"], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_reachable_set()
            .contains(&p.data.def_nr("n_target")),
        "fn-ref argument must be reachable (L4)"
    );
}

/// L4 — a fn-ref laundered through a `fn(...)`-typed local (`f = target`,
/// emitted as `Set(f, Int(def_nr))`) is still in the set.
#[test]
fn fnref_laundered_through_a_variable_is_reachable_l4() {
    let src = "fn target(n: integer) -> integer { n }\n\
               fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
               fn scripted() -> integer { f = target; apply(f, 5) }\n";
    let p = parse_with_sandbox(&["fn:scripted"], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert!(
        p.sandbox_reachable_set()
            .contains(&p.data.def_nr("n_target")),
        "fn-ref laundered through a variable must be reachable (L4)"
    );
}

/// @PLN86 1.4 — a sandboxed def reaching an EXTERNAL native bridge (its
/// `#native` symbol owned by a native package) is flagged; a `#native` symbol
/// with no external-package owner (a built-in op) is not.
/// `native_symbol_crates` is the discriminator `backfill_native_symbol_crates`
/// fills for package-owned symbols.
#[test]
fn reachable_external_ffi_bridge_is_flagged_local_native_is_not() {
    let src = "fn ext_fn() -> integer; #native \"ext_sym\"\n\
               fn local_native() -> integer; #native \"local_sym\"\n\
               fn scripted() -> integer { ext_fn(); local_native() }\n";
    let mut p = parse_with_sandbox(&["fn:scripted"], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    // Simulate `ext_sym` being owned by an external native package — exactly
    // what `backfill_native_symbol_crates` records for a def under a
    // `native_packages` dir.  `local_sym` is left unowned.
    p.data
        .native_symbol_crates
        .insert("ext_sym".to_string(), "extcrate".to_string());
    let bridges = p.sandbox_ffi_bridges();
    assert!(
        bridges.contains(&p.data.def_nr("n_ext_fn")),
        "reachable external FFI bridge must be flagged, got {bridges:?}"
    );
    assert!(
        !bridges.contains(&p.data.def_nr("n_local_native")),
        "a #native symbol with no external-package owner must NOT be flagged"
    );
}

/// @PLN86 — a `group#right` call-gate link parses off the signature onto a def
/// and is readable; the read/update distinction round-trips, and an unlinked def
/// reads as `None`.
#[test]
fn cap_annotation_is_parsed_and_readable() {
    let src = "fn reader() -> integer collections#read;\n#native\n\
               fn writer() -> integer collections#update;\n#native\n\
               fn plain() -> integer { 0 }\n";
    let p = parse_with_sandbox(&[], src);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    assert_eq!(
        p.def_cap_group(p.data.def_nr("n_reader")),
        Some("collections#read")
    );
    assert_eq!(
        p.def_cap_group(p.data.def_nr("n_writer")),
        Some("collections#update")
    );
    assert_eq!(p.def_cap_group(p.data.def_nr("n_plain")), None);
}
