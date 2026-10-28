// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PushRec`'s apart-call clause — the pins: which loops of the guard
//! `tests/scripts/a-vector-built-across-a-writing-call-keeps-its-elements.loft` hold a
//! record-push header across their writing call, and which the clause declines.  The guard
//! checks the values on both backends; this checks the FORM, so a clause that stops firing
//! (or fires where a value can reach the vector) is named by function.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-vector-built-across-a-writing-call-keeps-its-elements.loft";

/// Per function: `(loop record-push headers bound, appends through a header)`.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("n_rd", 2, 2),   // the node loop and the map loop, each across `rd`'s call to itself
    ("n_rdb", 1, 1),  // a loop that can `break`: the header's finish writes the length each time
    ("n_fill", 0, 0), // `w` can name the vector `v` (a5 hands the same vector twice)
    ("n_a3", 0, 1),   // the vector handed to the call: no loop header, the group's own after it
];

fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += usize::from(line.contains("§ V-t record push header"));
        row.1 += line.matches("push_record_hoisted_zero::<").count();
    }
    map
}

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--ship")
        .arg("--native-rust")
        .arg("--native-emit")
        .arg(out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    fa::read_to_string(out).expect("read the emitted Rust")
}

#[test]
fn each_loop_holds_the_header_predicted() {
    let out = std::env::temp_dir().join("loft_apart_mint_on.rs");
    let got = counts(&emit(&out, &[]));
    for (name, headers, hoisted) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            (*headers, *hoisted),
            "{name}: (loop record-push headers, appends through a header)"
        );
    }
    let _ = fa::remove_file(&out);
}

#[test]
fn the_switch_takes_every_apart_header_away() {
    let out = std::env::temp_dir().join("loft_apart_mint_off.rs");
    let rust = emit(&out, &[("LOFT_NO_APART_MINT", "1")]);
    assert_eq!(
        rust.matches("§ V-t record push header").count(),
        0,
        "LOFT_NO_APART_MINT=1 left a loop header in a loop that calls a writer"
    );
    let _ = fa::remove_file(&out);
}
