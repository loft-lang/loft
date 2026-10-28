// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN91 strand 1b — the loft-written interpreter (`selfhost/`), run through its own
//! `loft test` against a program image this harness emits with `loft --emit-image`.
//!
//! The image is the IR the Rust front end writes; the package reads it as the records
//! `selfhost/src/ir.loft` declares.  The harness hands the image's path to the loft tests in
//! `LOFT_SELFHOST_IMAGE` (and a file that is no image in `LOFT_SELFHOST_NOT_AN_IMAGE`).

use loft::file_access as fa;
use std::path::PathBuf;

fn loft() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// S0.3 — the image of a program with `fn twice` and `fn main` opens in loft as the IR, and
/// the loader finds both definitions; a file that is no image answers null.
#[test]
fn the_selfhost_tests_pass_on_an_emitted_image() {
    let tmp = std::env::temp_dir().join(format!("loft_selfhost_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("scratch");
    let program = tmp.join("prog.loft");
    fa::write(
        &program,
        "fn twice(n: integer) -> integer { n * 2 }\nfn main() { println(\"{twice(21)}\"); }\n",
    )
    .expect("program");
    let image = tmp.join("prog.store");
    let not_an_image = tmp.join("not.store");
    fa::write(&not_an_image, b"not a store").expect("not an image");

    let emitted = loft::platform::process::harness_command(loft())
        .arg("--emit-image")
        .arg(&image)
        .arg(&program)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft --emit-image");
    assert!(
        emitted.status.success(),
        "emit-image: {}",
        String::from_utf8_lossy(&emitted.stderr)
    );

    let tested = loft::platform::process::harness_command(loft())
        .arg("test")
        .current_dir(root().join("selfhost"))
        .env("LOFT_SELFHOST_IMAGE", &image)
        .env("LOFT_SELFHOST_NOT_AN_IMAGE", &not_an_image)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("run loft test");
    let _ = fa::remove_dir_all(&tmp);
    let out = String::from_utf8_lossy(&tested.stdout);
    let err = String::from_utf8_lossy(&tested.stderr);
    assert!(
        tested.status.success() && out.contains("2 passed"),
        "selfhost tests:\n{out}\n{err}"
    );
}
