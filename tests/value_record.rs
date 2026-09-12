// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-aa (`@FR-R-ValueRecord`) — a NO-HEAP RECORD returned BY VALUE: an admitted
//! function returns its record's fields as a Rust tuple (in registers, no return buffer,
//! no store round-trip) exactly as a TUPLE return already did, and its call sites read
//! tuple elements instead of a store.  The cell corpus
//! (`bytecode-comparisons/V-aa-value-record-cells.loft`) says the VALUES hold; this pins
//! the EMISSION — which functions are admitted, and that every declining shape (a stored
//! record, a heap field, too many fields, a passed-on or returned-onward result, a
//! forwarding body) keeps its buffer — and the switch (`LOFT_NO_VALUE_RECORD=1`).
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-aa-value-record-cells.loft";

/// `(function, returns a tuple?)` — the predictions written beside the cells.
const EXPECTED: &[(&str, bool)] = &[
    ("n_mk_smp", true),         // c1: every site reads fields
    ("n_mk_mixed", true),       // c2: mixed scalar field types
    ("n_mk_smp_kept", false),   // c3: a site STORES the record
    ("n_mk_owner", false),      // c4: a `text` field owns heap
    ("n_mk_big", false),        // c5: past the field-count bound
    ("n_mk_smp_passed", false), // c6: the result is passed on
    ("n_mk_cond", false),       // c7: the arms' Objects sit under an `if` the value
    // path does not rebuild — the body gate declines it (conservative; values exact)
    ("n_mk_inner", false),     // c8: its result builds another record
    ("n_mk_ret", false),       // c9: its caller RETURNS the record onward
    ("n_pass_through", false), // c9: a FORWARDING body has no Object to convert
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        // § V-aa is OPT-IN (see `hoist::value_record_disabled`): its gates do not hold
        // across the script corpus, so the default build keeps the return buffer.  These
        // tests are what the unit is developed against, so they ask for it explicitly.
        .env("LOFT_VALUE_RECORD", "1");
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
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// Does `rust` declare `name` with a TUPLE return (the value form)?
fn returns_tuple(rust: &str, name: &str) -> bool {
    rust.lines()
        .find(|l| l.starts_with(&format!("fn {name}(")))
        .map(|l| {
            let ret = l.split("->").nth(1).unwrap_or("");
            ret.trim_start().starts_with('(')
        })
        .unwrap_or_else(|| panic!("{name} was not emitted"))
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_returns_by_value_exactly_where_predicted() {
    let out = std::env::temp_dir().join("loft_value_record_on.rs");
    let rust = emit(&cells(), &out, &[]);
    for (name, want) in EXPECTED {
        assert_eq!(
            returns_tuple(&rust, name),
            *want,
            "{name}: returns its record by value"
        );
    }
    // An admitted callee takes no return buffer, and its call site passes none.
    assert!(
        !rust.contains(
            "fn n_mk_smp(cell: &std::cell::UnsafeCell<Stores>, mut var_v: f64, mut var___retbuf"
        ),
        "an admitted fn must not keep its __retbuf parameter"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_default_build_keeps_every_return_buffer() {
    let out = std::env::temp_dir().join("loft_value_record_off.rs");
    // The unit is OPT-IN, so asking for nothing is the off state: `emit` sets
    // LOFT_VALUE_RECORD=1 and this overrides it back to 0.  Pinning the DEFAULT here (not
    // just "a switch turns it off") is the point — it is what a user's build does.
    let rust = emit(&cells(), &out, &[("LOFT_VALUE_RECORD", "0")]);
    for (name, _) in EXPECTED {
        assert!(
            !returns_tuple(&rust, name),
            "{name}: the default build keeps its return buffer (§ V-aa is opt-in)"
        );
    }
    let _ = std::fs::remove_file(&out);
}
