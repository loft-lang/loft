// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1322 — a store is released ONCE (`formal/ownership.md` @FR-O-Borrow).
//!
//! A collection minted in a `??` default arm was freed twice: `_vec_N` and `__vdb_N` name one
//! store (the view is `OpGetField(__vdb_N, 0)`), the return-delivery materializer freed it
//! through the view after the append, and the record kept a scope-exit free of its own.
//!
//! **The verdict lives here rather than in the `.loft` guard because no VALUE can carry it.** A
//! second free of an already-freed store is a no-op — `free_named` returns — so the program
//! answers identically before and after, and `LOFT_STRICT_STORES` does not flag it either. The
//! only channel is `LOFT_TRACE_DB`'s `already_free=true` line, which is what these tests count.
//! The value half is `tests/scripts/1322-a-default-arm-mint-is-freed-once.loft`, run by the
//! ordinary corpus.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Write `src` to a scratch file and run it under `LOFT_TRACE_DB`, answering
/// `(stdout, redundant_free_count)`.
fn redundant_frees(name: &str, src: &str) -> (String, usize) {
    let dir = std::env::temp_dir().join(format!("loft_redundant_free_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join(format!("{name}.loft"));
    fa::write(&file, src).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TRACE_DB", "1")
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let count = stderr
        .lines()
        .filter(|l| l.contains("already_free=true"))
        .count();
    (String::from_utf8_lossy(&out.stdout).into_owned(), count)
}

const DEFAULT_ARM: &str = r#"
fn main() {
  g = fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] };
  none: vector<integer>? = null;
  print("{g(none)[1]}\n");
}
"#;

const KEYED_DEFAULT_ARM: &str = r#"
struct Row { k: integer, v: integer }
fn main() {
  g = fn(q: hash<Row[k]>?) -> integer { len(q ?? [Row { k: 1, v: 1 }]) };
  none: hash<Row[k]>? = null;
  print("{g(none)}\n");
}
"#;

const BORROW_ARM: &str = r#"
fn main() {
  g = fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] };
  some: vector<integer>? = [41, 42];
  print("{g(some)[1]}\n");
}
"#;

/// A closure that CAPTURES a collection, with no `??` anywhere.
///
/// This is the harness's proof that it can fail, and it is an honest one rather than a
/// deliberately-broken cell: the closure record is released through both the fn-ref value and
/// the `___clos_N` local, so its cascade runs twice and the second pass finds the capture's
/// store already gone. Same rule, different pair of names, and NOT what loft#1322 is about —
/// which is why this test asserts the count is non-zero rather than pretending otherwise.
const CAPTURED_COLLECTION: &str = r#"
fn main() {
  d: vector<integer> = [3];
  g = fn(k: integer) -> integer { d[0] };
  print("{g(0)}\n");
}
"#;

#[test]
fn a_default_arm_mint_is_released_once() {
    let (stdout, count) = redundant_frees("default_arm", DEFAULT_ARM);
    assert!(stdout.contains('8'), "the default arm's value: {stdout}");
    assert_eq!(
        count, 0,
        "`??` default arm released its store {count} time(s) too many"
    );
}

#[test]
fn a_keyed_default_arm_mint_is_released_once() {
    let (stdout, count) = redundant_frees("keyed_default_arm", KEYED_DEFAULT_ARM);
    assert!(
        stdout.contains('1'),
        "the keyed default arm's value: {stdout}"
    );
    assert_eq!(
        count, 0,
        "keyed `??` default arm released its store {count} time(s) too many"
    );
}

#[test]
fn the_borrow_arm_releases_nothing_twice() {
    let (stdout, count) = redundant_frees("borrow_arm", BORROW_ARM);
    assert!(stdout.contains("42"), "the borrow arm's value: {stdout}");
    assert_eq!(
        count, 0,
        "`??` borrow arm released a store {count} time(s) too many"
    );
}

#[test]
fn the_harness_can_see_a_redundant_free() {
    // Without a cell that DOES report one, every assertion above would hold on a build whose
    // trace said nothing at all — a broken instrument and a clean tree read alike.
    let (stdout, count) = redundant_frees("captured_collection", CAPTURED_COLLECTION);
    assert!(stdout.contains('3'), "the captured value: {stdout}");
    assert!(
        count > 0,
        "the instrument reported no redundant free for a captured collection — either the \
         closure-record double release has been fixed (good: replace this cell with a shape \
         that still shows one, or retire it) or `LOFT_TRACE_DB` has stopped reporting \
         `already_free=true` and the three tests above are vacuous"
    );
}
