// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN181 — the compiled standard library: `src/compiled_stdlib_gen.rs` is the standard
//! library's compiled loft bodies, generated with loft itself (`src/compiled_stdlib.rs`).
use loft::parser::Parser;
use loft::scopes;

fn generated() -> String {
    let mut p = Parser::new();
    // Unchecked, as `regen_fill_rs` is: the check compares `default/` against tables this
    // regeneration exists to repair.
    p.parse_dir_unchecked("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    loft::compiled_stdlib::generate(
        &p.data,
        &p.database,
        &loft::cache::collect_stdlib_sources("default"),
    )
}

/// The generated file is what the standard library generates today.
#[test]
fn compiled_stdlib_up_to_date() {
    let current = std::fs::read_to_string("src/compiled_stdlib_gen.rs")
        .expect("read src/compiled_stdlib_gen.rs");
    assert!(
        current == generated(),
        "src/compiled_stdlib_gen.rs is out of date — run: make compiled-stdlib"
    );
}

/// Regenerate `src/compiled_stdlib_gen.rs`.  Run with `make compiled-stdlib`.
#[test]
#[ignore = "maintenance: regenerates src/compiled_stdlib_gen.rs — `make compiled-stdlib`"]
fn regen_compiled_stdlib() {
    std::fs::write("src/compiled_stdlib_gen.rs", generated()).expect("write the generated file");
    println!("src/compiled_stdlib_gen.rs regenerated");
}
