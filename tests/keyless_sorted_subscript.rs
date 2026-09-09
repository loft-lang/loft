// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 (loft#1426, found writing the § V-m cells) — a subscript on a REFUSED keyed
//! collection is an error, not an internal compiler error.
//!
//! `sorted<integer>` declares no key, which `(Col-Sorted)` requires, and the declaration is
//! refused with "Expect token [".  A subscript on it then reached `Parser::parse_key` with an
//! EMPTY key list, and `key_types[0]` panicked — an ICE on a program that had already been
//! told what was wrong.  The CLI path is the one that reaches it (`loft --check`); the test
//! runner's recovery on the same source takes another route, so a corpus guard cannot see
//! the defect — this test runs the CLI and reads its stderr.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "fn main() { s: sorted<integer> = [5, 1, 3]; println(\"{s[0]?}\"); }\n";

#[test]
fn a_subscript_on_a_keyless_sorted_is_refused_without_an_ice() {
    let src = std::env::temp_dir().join("loft_keyless_sorted_subscript.loft");
    std::fs::write(&src, PROBE).expect("write probe");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--check")
        .arg(&src)
        .env("LOFT_TIMEOUT", "60")
        .output()
        .expect("spawn loft --check");
    let err = String::from_utf8_lossy(&out.stderr);
    let all = format!("{err}{}", String::from_utf8_lossy(&out.stdout));
    assert!(
        !out.status.success(),
        "a keyless sorted must not compile:\n{all}"
    );
    assert!(
        all.contains("Expect token ["),
        "the declaration's own diagnostic must be what the reader sees:\n{all}"
    );
    assert!(
        !all.contains("internal compiler error") && !all.contains("index out of bounds"),
        "the refused collection's subscript must not be an ICE:\n{all}"
    );
    let _ = std::fs::remove_file(&src);
}
