// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#962 — a package's `const` initialised from a sibling module's `const`.
//!
//! The single-file half of this bug is pinned in
//! `tests/scripts/962-const-initialised-from-a-later-name.loft`, which is where the
//! mechanism is written down: `parse_constant` stored a constant's IR on PASS 1 only, and
//! pass 1 resolves names against an incomplete definition table by construction.
//!
//! This file pins the shape it was REPORTED as, which no single-file test can reach and
//! which is what made it expensive: the consumer boundary.  Compiling the library's own
//! aggregator is completely clean — no diagnostic at all — so the library looks healthy,
//! and every program that `use`s it panics with `index out of bounds: the len is 1 but the
//! index is 65535`, pointing at an unrelated function's return type in a file the consumer
//! did not write.  In dryopea that was the difference between `--native-emit` on the
//! library (silent) and `loft test` (panic on the first test file).
//!
//! ⚠ THE IMPORT SPELLING IS THE AXIS, and it is invisible in the source.  `src/two.loft`
//! reaches its sibling through the package AGGREGATOR (`use repro;`) rather than directly
//! (`use one;`).  The direct spelling works and always did; the aggregator spelling is the
//! one that matches every other file in a conventional package.  A fixture written with
//! `use one;` would pass with the fix reverted, so the control below writes exactly that
//! and asserts it too — one axis moved, everything else held.

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// A three-module package: an aggregator, a module owning a `const`, and a module whose own
/// `const` is initialised FROM it.  `import` is the line under test at the top of
/// `src/two.loft`.
fn build_package(root: &Path, import: &str) {
    write(
        &root.join("repro/loft.toml"),
        "[package]\nname = \"repro\"\nversion = \"0.1.0\"\n[library]\nentry = \"src/repro.loft\"\n",
    );
    write(
        &root.join("repro/src/repro.loft"),
        "pub use one::*;\npub use two::*;\n",
    );
    write(
        &root.join("repro/src/one.loft"),
        "pub const ONE_PER_SECOND: integer = 3000000;\n",
    );
    write(
        &root.join("repro/src/two.loft"),
        &format!(
            "{import}\n\npub const TWO_WHOLE: integer = ONE_PER_SECOND * 1000000;\n\n\
             pub fn two_use(n: integer) -> integer {{ return (n * TWO_WHOLE) / 7; }}\n"
        ),
    );
    write(
        &root.join("entry.loft"),
        "use repro::*;\nfn main() { print(\"{two_use(2)}\\n\"); }\n",
    );
}

/// Compile and run the consumer entry, returning its combined output plus whether it
/// succeeded.
fn run_consumer(tag: &str, import: &str) -> (String, bool) {
    let root = std::env::temp_dir().join(format!("loft_962_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    build_package(&root, import);

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--lib")
        .arg(&root)
        .arg(root.join("entry.loft"))
        .env("LOFT_TIMEOUT", "60")
        .current_dir(root.join("repro"))
        .output()
        .expect("spawn loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let ok = out.status.success();
    let _ = std::fs::remove_dir_all(&root);
    (all, ok)
}

/// The reported case.  `2 * 3000000 * 1000000 / 7` is asserted as a VALUE, not as the
/// absence of a panic: a constant that resolved to the wrong number would be a quieter
/// version of the same defect, and the silent cells of this bug (a text constant reading
/// empty, a call-initialised one reading `null`) are exactly that.
#[test]
fn a_const_from_a_sibling_module_reaches_the_consumer() {
    let (all, ok) = run_consumer("agg", "pub use repro::*;");
    assert!(
        all.contains("857142857142"),
        "the consumer must compute the constant, not panic on it.\n{all}"
    );
    assert!(
        !all.contains("65535"),
        "the file-scope no-slot sentinel must not reach codegen.\n{all}"
    );
    assert!(ok, "the consumer run must succeed.\n{all}");
}

/// The control: the SAME package with the sibling imported directly.  This spelling
/// already worked, and it is what keeps the test above honest — without it, a change that
/// broke cross-module constants outright would still leave that assertion looking specific.
#[test]
fn the_direct_import_spelling_keeps_working() {
    let (all, ok) = run_consumer("direct", "pub use one::*;");
    assert!(
        all.contains("857142857142"),
        "importing the sibling directly must be unaffected.\n{all}"
    );
    assert!(ok, "the control run must succeed.\n{all}");
}
