// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The keyed fast paths (@PLN158, `bench/portal/analysis/keyed.md` L1–L4) answer what the
//! general paths answer.
//!
//! `tests/scripts/158-keyed-fast-paths.loft` states every answer by hand, and the corpus
//! runners already run it as built on both backends.  That leaves two readings nothing else
//! takes, and this file takes them:
//!
//! * with every switch of the pass set (`LOFT_NO_FAST_ORDER`, `LOFT_NO_ONE_PROBE_INSERT`,
//!   `LOFT_NO_HALF_LOAD`, `LOFT_NO_TYPED_KEYED`) — the GENERAL paths must give the same
//!   hand-computed answers, or the cells would be pinning the fast paths to themselves;
//! * with `LOFT_KEYED_VERIFY=1` — every pre-resolved comparison, every exact `index`
//!   lookup and every one-probe `hash` insert is checked against the general form as it
//!   is made, and a disagreement panics.  The cells are the workload; the check is per
//!   operation, so it sees a wrong comparison that happens not to change a cell's answer.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-keyed-fast-paths.loft";

fn run(backend: &str, env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(backend)
        .arg("--tests")
        .arg(&src)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_FAST_ORDER")
        .env_remove("LOFT_NO_ONE_PROBE_INSERT")
        .env_remove("LOFT_NO_HALF_LOAD")
        .env_remove("LOFT_NO_TYPED_KEYED")
        .env_remove("LOFT_KEYED_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft --tests");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && text.contains("test result: ok. 14 passed"),
        "{backend} {env:?}: the keyed cells did not all pass\n{text}"
    );
}

const GENERAL: [(&str, &str); 4] = [
    ("LOFT_NO_FAST_ORDER", "1"),
    ("LOFT_NO_ONE_PROBE_INSERT", "1"),
    ("LOFT_NO_HALF_LOAD", "1"),
    ("LOFT_NO_TYPED_KEYED", "1"),
];
const VERIFY: [(&str, &str); 1] = [("LOFT_KEYED_VERIFY", "1")];

#[test]
fn the_general_paths_give_the_same_answers_on_the_interpreter() {
    run("--interpret", &GENERAL);
}

#[test]
fn the_general_paths_give_the_same_answers_on_native() {
    run("--native", &GENERAL);
}

#[test]
fn every_fast_answer_verifies_against_the_general_one_on_the_interpreter() {
    run("--interpret", &VERIFY);
}

#[test]
fn every_fast_answer_verifies_against_the_general_one_on_native() {
    run("--native", &VERIFY);
}

/// The Rust `--native` emits for the cells, with `env` set.
fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_keyed_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_TYPED_KEYED");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// `@FR-R-TypedKeyed` — the EMISSION pin.  A lookup in a `hash` whose one key is an
/// integer width takes the typed entry; an `index`, a text key, a compound key and a width
/// the fast forms do not list keep the general call; the switch restores it everywhere.
#[test]
fn a_hash_lookup_by_one_integer_key_takes_the_typed_entry() {
    let rust = emit("typed", &[]);
    // (function, typed lookups, general lookups)
    for (name, typed, general) in [
        ("n_hv", 1, 0),                                // hash<E[id]>, integer
        ("n_iv", 0, 1),                                // index<I[id]>: not a hash
        ("n_test_158_hash_text_find_or_insert", 0, 6), // text key
        ("n_test_158_hash_compound_key", 0, 5),        // two key fields
        ("n_test_158_hash_in_a_group", 3, 0),          // a hash in a group is still a hash
    ] {
        let b = body(&rust, name);
        assert_eq!(
            b.matches("OpGetHashLong(").count(),
            typed,
            "{name}: typed lookups"
        );
        assert_eq!(
            b.matches("OpGetRecord(").count(),
            general,
            "{name}: general lookups"
        );
    }
    // `u8` is not a width `find_long` lists; `i32` is, and the index beside it is general.
    let narrow = body(&rust, "n_test_158_hash_narrow_keys");
    assert_eq!(
        narrow.matches("OpGetHashLong(").count(),
        3,
        "i32 hash lookups"
    );
    assert_eq!(
        narrow.matches("OpGetRecord(").count(),
        3,
        "u8 hash + i32 index lookups"
    );

    let off = emit("general", &[("LOFT_NO_TYPED_KEYED", "1")]);
    assert_eq!(
        off.matches("OpGetHashLong(").count(),
        0,
        "LOFT_NO_TYPED_KEYED=1 emits the general lookup everywhere"
    );
    assert_eq!(
        off.matches("OpGetRecord(").count(),
        rust.matches("OpGetRecord(").count() + rust.matches("OpGetHashLong(").count(),
        "and nothing else moved"
    );
}
