// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The interpreter emits no `OpPreAllocVector` that only repeats the append after it — PINNED
//! on the bytecode of the cells (`tests/scripts/a-literal-append-claims-its-vector-once.loft`):
//! a reservation for a literal of at most 11 elements is gone, a wider one stays, and
//! `LOFT_NO_PREALLOC_ELIDE=1` keeps every one.  The cells' values pass on either form, so only
//! this pin sees the elision drift.  The IR and the native Rust do not change, which the
//! third test checks: the op is the head `--native`'s append-group recognisers read.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-literal-append-claims-its-vector-once.loft";

/// Per function: the reservations in its bytecode with the elision ON, and with it OFF.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_ints", 0, 1),                   // the counted push loop the elision is for
    ("n_test_narrow_bytes", 0, 1),      // a `u8` element: one byte wide
    ("n_test_records", 0, 1),           // a record element
    ("n_test_nested", 0, 2),            // a nested vector and its row literal
    ("n_test_text", 0, 1),              // a text element
    ("n_test_eleven_and_twelve", 1, 4), // eleven elided, twelve kept
    ("n_test_cleared_refill", 0, 2),    // the vector has a record on the second fill
    ("n_test_array", 0, 1),             // record-id slots: each element its own record
    ("n_test_field", 0, 0),             // a field: the parser reserves nothing
];

fn introspect(env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect")
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_PREALLOC_ELIDE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        text.contains("fn n_ints("),
        "no IR printed (exit {:?}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    text
}

/// The bytecode listing of `name`: from its `byte-code for …:name(` header to the next
/// header or the next printed function.
fn bytecode_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let head = format!(":{name}(");
    let start = dump
        .match_indices("byte-code for ")
        .map(|(i, _)| i)
        .find(|&i| dump[i..].lines().next().is_some_and(|l| l.contains(&head)))
        .unwrap_or_else(|| panic!("no bytecode printed for {name}"));
    let rest = &dump[start + 1..];
    let end = [rest.find("byte-code for "), rest.find("\nfn ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

fn reservations(code: &str) -> usize {
    code.matches("PreAllocVector(").count()
}

#[test]
fn a_reservation_the_append_repeats_is_not_emitted() {
    let dump = introspect(&[]);
    for (name, on, _) in EXPECTED {
        assert_eq!(
            reservations(bytecode_of(&dump, name)),
            *on,
            "{name}: reservations in the bytecode — the elision moved"
        );
    }
}

#[test]
fn the_switch_keeps_every_reservation() {
    let dump = introspect(&[("LOFT_NO_PREALLOC_ELIDE", "1")]);
    for (name, _, off) in EXPECTED {
        assert_eq!(
            reservations(bytecode_of(&dump, name)),
            *off,
            "{name}: reservations with the elision off"
        );
    }
}

/// Everything but the bytecode — the IR and the native Rust — is the same either way.
#[test]
fn the_ir_and_the_native_rust_do_not_change() {
    let strip = |s: &str| -> String {
        s.lines()
            .filter(|l| {
                let t = l.trim_start();
                !(t.starts_with("byte-code for ")
                    || t.starts_with(":POS")
                    || t.split_once('[').is_some_and(|(n, r)| {
                        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && r.contains("]:")
                    }))
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let on = strip(&introspect(&[]));
    let off = strip(&introspect(&[("LOFT_NO_PREALLOC_ELIDE", "1")]));
    assert!(
        on.contains("fn n_ints("),
        "the IR was stripped with the bytecode"
    );
    assert!(
        on == off,
        "the IR or the native Rust differs with the elision off"
    );
}
