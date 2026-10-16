// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

use super::Parser;
use crate::sandbox::parse_sandbox_config;

fn sandboxed_parser() -> Parser {
    let mut p = Parser::new();
    p.set_sandbox_config(parse_sandbox_config(
        "[sandbox]\nmod-script = [\"fn:scripted\"]\n[profile.mod-script]\nallow = [\"math#read\"]\n",
    ));
    p
}

fn parse_source(p: &mut Parser, src: &str) {
    // Process-global counter (not the `src` pointer) for a collision-free name
    // across concurrent test threads — see `parse_admit_libs`.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "plan86_nest_{}_{}.loft",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    crate::file_access::write(&path, src).unwrap();
    p.parse(path.to_str().unwrap(), false);
    let _ = crate::file_access::remove_file(&path);
}

#[test]
fn capability_declarations_register_and_resolve() {
    // @PLN86 P6.1 — a `capability` decl registers its dotted name; a
    // `group#right` link resolves against it, and an undeclared group does not
    // (the load error a mistyped link becomes).  The dotted name is the namespace.
    let mut p = Parser::new();
    parse_source(
        &mut p,
        "capability fs\ncapability cmd.move\nfn main() { }\n",
    );
    assert!(p.cap_is_declared("fs"), "fs should be declared");
    assert!(
        p.cap_is_declared("cmd.move"),
        "the dotted name cmd.move should be declared"
    );
    assert!(
        !p.cap_is_declared("typo"),
        "an undeclared group must not resolve"
    );
    // the `#right` suffix parses to exactly the three rights, nothing else.
    use crate::sandbox::Right;
    assert_eq!(Right::parse("read"), Some(Right::Read));
    assert_eq!(Right::parse("update"), Some(Right::Update));
    assert_eq!(Right::parse("append"), Some(Right::Append));
    assert_eq!(Right::parse("delete"), None);
}

#[test]
fn field_capability_links_are_recorded() {
    // @PLN86 P6.4 — a `group#right` link after a struct field's type is recorded
    // per (struct, field); an unlinked field carries none; multiple links stack;
    // append rides on a collection field.
    let mut p = Parser::new();
    parse_source(
        &mut p,
        "capability stats\ncapability bag\n\
         struct Item { v: integer }\n\
         struct Entity { id: integer, health: integer stats#read stats#update, \
         loot: vector<Item> bag#read bag#append }\n\
         fn main() { }\n",
    );
    let e = p.data.def_nr("Entity");
    let links = |f: &str| -> Vec<String> { p.member_links(e, f).to_vec() };
    assert!(links("id").is_empty(), "an unlinked field carries no links");
    assert_eq!(links("health"), ["stats#read", "stats#update"]);
    assert_eq!(links("loot"), ["bag#read", "bag#append"]);
}

/// @PLN86 0.1 — hostile deep nesting inside a sandboxed def is a clean
/// LOAD-time parse error, NOT a native stack overflow (rc=139).  The
/// definitive check is the pair of asserts below (a depth error was emitted
/// AND it is the nesting-depth diagnostic); the explicit stack size only has
/// to be large enough that the process REACHES those asserts before the guard
/// trips at 128 — a stack overflow aborts the whole process, it is not a
/// catchable panic.
///
/// Sizing: one nesting level costs ≈15 KB of native stack (measured; the
/// `expression → operators → part → single` chain — the null-flow parse logic
/// in #559 grew it from ≈10 KB), so 128 levels ≈ 1.8 MB uninstrumented.  The
/// ASan gate inflates every frame ≈4.4× (redzones + shadow), pushing 128
/// levels to ≈8–9 MB — which overflowed the former 8 MB thread here and turned
/// the ASan gate red (a guard that fires correctly, starved of stack).  64 MB
/// clears 128 ASan-inflated levels with margin, while a runaway (broken guard →
/// 2000 levels ≈ 128 MB under ASan) still overflows, so a BROKEN guard is caught
/// either by the asserts or by an overflow.
#[test]
fn deep_nesting_in_sandboxed_def_is_a_clean_error_not_a_crash() {
    let (has_error, has_msg) = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let depth = 2000; // >> crate::limits::LIMIT and into the overflow zone
            let src = format!(
                "fn scripted() {{ x = {}1{}; }}\n",
                "(".repeat(depth),
                ")".repeat(depth)
            );
            let mut p = sandboxed_parser();
            parse_source(&mut p, &src);
            let has_error = p.diagnostics.level() >= crate::diagnostics::Level::Error;
            let has_msg = p
                .diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("nesting too deep"));
            (has_error, has_msg)
        })
        .unwrap()
        .join()
        .expect("the parse thread must not overflow/panic");
    assert!(has_error, "expected a depth error");
    assert!(has_msg, "expected the nesting-depth diagnostic");
}

/// The guard must not false-trip: ordinary nesting well below the limit —
/// and many sibling expressions — parse cleanly (proves the depth counter is
/// balanced: each sub-expression decrements, so siblings don't accumulate).
#[test]
fn ordinary_nesting_in_sandboxed_def_parses_clean() {
    use std::fmt::Write as _;
    let mut body = String::from("fn scripted() {\n");
    for i in 0..200 {
        let _ = writeln!(body, "  a{i} = ((({i})));"); // shallow, 200 siblings
    }
    body.push_str("}\n");
    let mut p = sandboxed_parser();
    parse_source(&mut p, &body);
    assert!(
        !p.diagnostics
            .lines()
            .iter()
            .any(|l| l.contains("nesting too deep")),
        "guard false-tripped on ordinary code: {:?}",
        p.diagnostics.lines()
    );
}
