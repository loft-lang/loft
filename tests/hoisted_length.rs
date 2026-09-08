// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 (loft#1426) — a loop that hoisted a vector's header reads the loop BOUND from
//! that header, not from the store table on every iteration.
//!
//! The end-to-end guard cannot see this (both forms answer the same numbers); what
//! changed is the emitted Rust, so the three shapes are pinned here: a loop that writes
//! no store reads `__vh_N.len`; a loop that appends keeps the runtime read, because it
//! hoisted nothing; and a length read outside any loop keeps it too.  Measured: the
//! rewrite alone halves a 200 000-element sum loop (645k → 330k ns/op).
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
fn sum_hoisted(xs: const vector<integer>) -> integer { s = 0; for x in xs { s = s + x; } s }
fn copy_appending(xs: const vector<integer>) -> vector<integer> { out: vector<integer> = []; for x in xs { out += [x + 1]; } out }
fn count(xs: const vector<integer>) -> integer { len(xs) * 2 }
fn main() { v: vector<integer> = [3, 5, 7]; println(\"{sum_hoisted(v)} {len(copy_appending(v))} {count(v)}\"); }
";

fn emit(src: &std::path::Path, out: &std::path::Path) -> String {
    let status = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--native-emit")
        .arg(out)
        .arg("--lean")
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        .status()
        .expect("spawn loft --native-emit");
    assert!(status.success(), "loft --native-emit failed");
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// The emitted Rust of one function.
fn fn_of<'a>(rs: &'a str, name: &str) -> &'a str {
    let start = rs
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no emitted fn {name}"));
    let rest = &rs[start..];
    let end = rest.find("\n}").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn a_hoisted_loop_reads_its_bound_from_the_header_and_the_others_keep_the_runtime_read() {
    let src = std::env::temp_dir().join("loft_hoisted_length_probe.loft");
    let out = std::env::temp_dir().join("loft_hoisted_length_probe.rs");
    std::fs::write(&src, PROBE).expect("write probe");
    let rs = emit(&src, &out);
    let hoisted = fn_of(&rs, "n_sum_hoisted");
    assert!(
        hoisted.contains("vec_header(") && hoisted.contains("(i64::from(__vh_1.len))"),
        "the hoisted loop's bound must read the header's length:\n{hoisted}"
    );
    // The emitter also spells a FALLBACK copy of the loop for the path where no header
    // could be derived; that copy keeps the runtime read by design, so at most one remains.
    assert!(
        hoisted.matches("length_vector(").count() <= 1,
        "only the fallback copy may still read the length at run time:\n{hoisted}"
    );
    let appending = fn_of(&rs, "n_copy_appending");
    assert!(
        !appending.contains("vec_header(") && appending.contains("length_vector("),
        "an appending loop hoists nothing and keeps the runtime read:\n{appending}"
    );
    let outside = fn_of(&rs, "n_count");
    assert!(
        !outside.contains("__vh_") && !outside.contains(".len))"),
        "a length read outside a loop has no header to read:\n{outside}"
    );
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);
}
