// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN181 — the compiled standard library: `src/compiled_stdlib_gen.rs` is the standard
//! library's compiled loft bodies, generated with loft itself (`src/stdlib_codegen.rs`).
use loft::file_access as fa;
use loft::parser::Parser;
use loft::scopes;

fn generated() -> String {
    let mut p = Parser::new();
    // Unchecked, as `regen_fill_rs` is: the check compares `default/` against tables this
    // regeneration exists to repair.
    p.parse_dir_unchecked("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    loft::stdlib_codegen::generate(
        &p.data,
        &p.database,
        &loft::cache::collect_stdlib_sources("default"),
    )
}

/// The generated file is what the standard library generates today.
#[test]
fn compiled_stdlib_up_to_date() {
    let current =
        fa::read_to_string("src/compiled_stdlib_gen.rs").expect("read src/compiled_stdlib_gen.rs");
    assert!(
        current == generated(),
        "src/compiled_stdlib_gen.rs is out of date — run: make compiled-stdlib"
    );
}

/// Regenerate `src/compiled_stdlib_gen.rs`.  Run with `make compiled-stdlib`.
#[test]
#[ignore = "maintenance: regenerates src/compiled_stdlib_gen.rs — run with `--ignored` via `make compiled-stdlib`"]
fn regen_compiled_stdlib() {
    // Written only when it changed: cargo fingerprints the file by its time stamp, so an
    // identical rewrite rebuilds the whole release crate after it (`make compiled-stdlib`'s
    // own build, and every cargo step of a derive run).
    let text = generated();
    if fa::read_to_string("src/compiled_stdlib_gen.rs").is_ok_and(|old| old == text) {
        println!("src/compiled_stdlib_gen.rs unchanged");
        return;
    }
    fa::write("src/compiled_stdlib_gen.rs", text).expect("write the generated file");
    println!("src/compiled_stdlib_gen.rs regenerated");
}

/// What a module declares for the rest of the crate: each `pub(crate)` item, up to its value.
fn exports(source: &str) -> Vec<String> {
    let mut items: Vec<String> = source
        .lines()
        .filter(|l| l.starts_with("pub(crate) "))
        .map(|l| l.split(" = ").next().unwrap_or(l).to_string())
        .collect();
    items.sort();
    items
}

/// The stub a falsification control starts from (`scripts/falsify.sh`, GUARDS.md § The patch
/// receipt) declares exactly what the generated file exports, so a crate that builds with one
/// builds with the other.
#[test]
fn the_control_stub_declares_what_the_generated_file_exports() {
    let stub = fa::read_to_string("scripts/compiled_stdlib_stub.rs").expect("read the stub");
    let current =
        fa::read_to_string("src/compiled_stdlib_gen.rs").expect("read src/compiled_stdlib_gen.rs");
    assert!(!exports(&stub).is_empty(), "the stub declares nothing");
    assert_eq!(exports(&stub), exports(&current));
}
