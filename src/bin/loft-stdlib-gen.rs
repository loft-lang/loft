// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `make compiled-stdlib` (@PLN181): regenerate `src/compiled_stdlib_gen.rs` from `default/`,
//! written only when its text changed.  A binary rather than a test so it links the release
//! library the `loft` binary was built from: the test route built the library a second time,
//! with the dev-dependencies' features.  Run from the repository root.
fn main() {
    match loft::stdlib_codegen::regenerate("default", "src/compiled_stdlib_gen.rs") {
        Ok(wrote) => {
            println!(
                "src/compiled_stdlib_gen.rs {}",
                if wrote { "regenerated" } else { "unchanged" }
            );
        }
        Err(e) => {
            eprintln!("loft-stdlib-gen: {e}");
            std::process::exit(1);
        }
    }
}
