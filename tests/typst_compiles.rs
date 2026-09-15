// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Every generated Typst source still compiles.
//!
//! `doc/loft-reference.typ` is produced by `gendoc` and `doc/web-stack.typ` by
//! `scripts/md2typ.py`, and both are committed alongside the PDF they build. Nothing else
//! in the suite reads them, so a generator that emits markup Typst refuses produces a file
//! that looks fine in review, passes every gate, and fails the first time someone runs
//! `make pdf`.
//!
//! That is not hypothetical: a Typst escaper existed in two places and drifted by one
//! character class. The copy that fed the reference did not escape `_`, so a feature list
//! containing `log_*` emitted an emphasis delimiter that never closed and the whole
//! reference stopped compiling — with `make ci` green throughout, because `typst` is not
//! installed on a build box and nothing asked it.
//!
//! Skips cleanly when `typst` is absent, the way the native tests skip without `rustc`. A
//! skip is honest here: the check belongs wherever the PDFs are actually built.

use std::path::Path;
use std::process::Command;

fn typst_available() -> bool {
    Command::new("typst").arg("--version").output().is_ok()
}

/// Compile one generated `.typ` to a throwaway PDF and report the first error.
fn compiles(source: &str) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(source);
    if !src.exists() {
        println!("skip {source}: not generated in this tree");
        return;
    }
    // Under the tree, not the system temp dir: a snap-installed `typst` is confined
    // to the home directory and answers "Permission denied" for a PDF in `/tmp`,
    // while `make pdf` writing into `doc/` works on the same box.
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&scratch).expect("target/ is writable");
    let out = scratch.join(format!(
        "loft_typst_check_{}_{}.pdf",
        std::process::id(),
        source.replace(['/', '.'], "_")
    ));
    let result = Command::new("typst")
        .arg("compile")
        .arg(&src)
        .arg(&out)
        .output()
        .expect("typst runs once its presence is established");
    let _ = std::fs::remove_file(&out);
    assert!(
        result.status.success(),
        "{source} no longer compiles — `make pdf` would fail:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn every_generated_typst_source_still_compiles() {
    if !typst_available() {
        eprintln!(
            "skip: typst unavailable (the PDF sources are only checked where they are built)"
        );
        return;
    }
    for source in ["doc/loft-reference.typ", "doc/web-stack.typ"] {
        compiles(source);
    }
}
