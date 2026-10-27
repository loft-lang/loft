// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN181 — the compiled standard library: `src/compiled_stdlib_gen.rs` is the standard
//! library's compiled loft bodies, generated with loft itself (`src/stdlib_codegen.rs`).
use loft::file_access as fa;

fn generated() -> String {
    loft::stdlib_codegen::generate_from_dir("default").expect("read default/")
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

/// Regenerate `src/compiled_stdlib_gen.rs` from a test binary: `scripts/falsify.sh` does, for a
/// control whose patch reverts the runtime, which may not build the `loft-stdlib-gen` binary
/// `make compiled-stdlib` runs.
#[test]
#[ignore = "maintenance: regenerates src/compiled_stdlib_gen.rs — `make compiled-stdlib` is the usual route"]
fn regen_compiled_stdlib() {
    let wrote = loft::stdlib_codegen::regenerate("default", "src/compiled_stdlib_gen.rs")
        .expect("regenerate src/compiled_stdlib_gen.rs");
    println!(
        "src/compiled_stdlib_gen.rs {}",
        if wrote { "regenerated" } else { "unchanged" }
    );
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
