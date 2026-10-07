// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `(R-Header)`'s FUNCTION clause — a function body holds one header per vector PARAMETER
//! it reads twice or more and leaves as it found it (`hoist::param_untouched`), bound at
//! entry after the prologue; the cells
//! (`tests/scripts/a-function-reads-its-vector-parameter-through-one-header.loft`) say
//! the values hold on both backends and under `LOFT_HOIST_VERIFY=1`.  This pins the
//! EMISSION per cell — which parameters earn a header, which decline — and the switch
//! (`LOFT_NO_FN_HEADER=1`), read off `--native-emit`.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-function-reads-its-vector-parameter-through-one-header.loft";

/// `(function, headers bound at entry)` — the predictions written beside the cells.
const EXPECTED: &[(&str, usize)] = &[
    ("n_f1", 1),  // a parameter read three times
    ("n_f2", 0),  // read once: nothing to gain
    ("n_f3", 1),  // the body pushes to a LOCAL (a work buffer): another store
    ("n_f4", 1),  // a recursive reader: the recursion answers for itself
    ("n_f5", 1),  // a nullable parameter: a null header has length 0
    ("n_f6", 1),  // a read behind a branch
    ("n_f7", 2),  // two parameters
    ("n_f8", 0),  // a local built then read: not a parameter, and written
    ("n_f9", 0),  // the parameter is rebound
    ("n_f10", 0), // the parameter is pushed to
    ("n_f11", 1), // a slice of the parameter only reads it
];

fn emit(env: &[(&str, &str)]) -> String {
    // One file per test: the two run in parallel under one process id.
    let out = std::env::temp_dir().join(format!(
        "loft_fn_header_{}_{}.rs",
        std::process::id(),
        if env.is_empty() { "on" } else { "off" }
    ));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_FN_HEADER")
        .env_remove("LOFT_NO_VECTOR_HOIST");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let text = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
    text
}

/// The emitted body of function `name`: from its `fn` line to the next top-level `fn`.
fn body_of<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &text[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

#[test]
fn each_parameter_earns_exactly_the_header_predicted() {
    let text = emit(&[]);
    for (name, headers) in EXPECTED {
        let body = body_of(&text, name);
        assert_eq!(
            body.matches("vec_header(").count(),
            *headers,
            "{name}: headers bound at entry"
        );
        assert_eq!(
            body.matches("//@FR-R-Header function clause").count(),
            *headers,
            "{name}: every header is the function clause's"
        );
    }
}

#[test]
fn the_switch_binds_no_function_header() {
    let text = emit(&[("LOFT_NO_FN_HEADER", "1")]);
    assert!(
        !text.contains("//@FR-R-Header function clause"),
        "LOFT_NO_FN_HEADER=1 must bind no header at a function's entry"
    );
    for (name, _) in EXPECTED {
        let body = body_of(&text, name);
        assert_eq!(
            body.matches("vec_header(").count(),
            0,
            "{name}: no header under the switch"
        );
    }
}
