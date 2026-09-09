// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-p (loft#1426) — a callee's INVARIANT INPUTS cross the call (`@FR-R-Inputs`):
//! a callee admitted by `@FR-R-Callee` gets a TWIN (`<fn>__inv`) that takes the record
//! parameter's invariant scalars and vector headers as extra parameters, and a caller loop
//! that hoisted every one of them for its argument variable calls the twin.
//!
//! The cell corpus (`bytecode-comparisons/V-p-callee-inputs-cells.loft`) can only say the
//! VALUES hold — a twin and the plain call answer the same number.  This pins the EMISSION:
//! which callees earn a twin, which calls take it, and the switch
//! (`LOFT_NO_CALLEE_INPUTS=1`), which is what makes it red on the build before the unit and
//! on one that lost it.  Read off `--native-emit`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-p-callee-inputs-cells.loft";

/// The callees that earn a twin, with the twin's extra parameters in order.
const TWINS: &[(&str, &str)] = &[
    // the pixel methods: width, height, the header of `data`
    (
        "t_2Cv_getp",
        "__is_0: i64, __is_1: i64, __ih_0: vector::VecHeader",
    ),
    (
        "t_2Cv_setp",
        "__is_0: i64, __is_1: i64, __ih_0: vector::VecHeader",
    ),
    // c8: two record parameters — `dst.width`, then `src.data` indexed, `dst.data` viewed
    (
        "n_blend",
        "__is_0: i64, __ih_0: vector::VecHeader, __ih_1: vector::VecHeader",
    ),
    // c10: a boolean and a float input
    (
        "t_2Ph_samp",
        "__is_0: u8, __is_1: f64, __ih_0: vector::VecHeader",
    ),
    // c12: transitive — getp2's inputs are getp's, re-spelled over its own `self`
    (
        "t_2Cv_getp2",
        "__is_0: i64, __is_1: i64, __ih_0: vector::VecHeader",
    ),
    // c14: a free function whose whole body is one field read
    ("n_readw", "__is_0: i64"),
];

/// The callees that earn NO twin, and why.
const NO_TWIN: &[&str] = &[
    "t_2Cv_bumpw", // writes the field it reads
    "n_setw",      // writes only
    "t_2Cv_cnt",   // a length-only view earns no header
    "t_2Cv_walk",  // recursive: not admitted
    "t_3Box_area", // reads through a nested record path
    "n_poke",      // writes only
];

/// `(caller, twin calls in its body)` — the predictions written beside the cells.
const CALLS: &[(&str, usize)] = &[
    ("n_c1", 2),             // getp + setp
    ("n_c2", 0),             // bumpw has no twin
    ("n_c3", 0),             // the loop writes the field: no hoist, no twin
    ("n_c4", 0),             // setw's write evicts width
    ("n_c5", 0),             // a field path, not a variable, as the argument
    ("n_c6", 0),             // an optional element view as the argument
    ("n_c7", 2),             // two records, two twin calls
    ("n_c8", 1),             // blend
    ("n_c9", 1), // outside a loop (plain), and a loop whose growth is a fusable PUSH — hoisted since § V-q
    ("n_c10", 2), // one per loop
    ("n_c11", 0), // cnt
    ("n_c12", 1), // getp2's twin …
    ("t_2Cv_getp2__inv", 1), // … which calls getp's twin
    ("n_c13", 1), // through a `&` alias
    ("n_c14", 1), // readw
    ("n_c15", 0), // walk
    ("n_c16", 0), // area
    ("n_c17", 1), // getp beside a writer of another type
    ("n_c18", 0), // a `&` view rebound in the loop
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120");
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

/// Per emitted function: its signature line, and how many twin CALLS its body makes.
fn functions(rust: &str) -> HashMap<String, (String, usize)> {
    let mut map: HashMap<String, (String, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.insert(current.clone(), (line.to_string(), 0));
            continue;
        }
        if line.contains("__inv(cell") {
            map.entry(current.clone()).or_default().1 += 1;
        }
    }
    map
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_callee_earns_exactly_the_twin_predicted_and_each_call_takes_it() {
    let out = std::env::temp_dir().join("loft_callee_inputs_on.rs");
    let fns = functions(&emit(&cells(), &out, &[]));
    for (name, params) in TWINS {
        let twin = format!("{name}__inv");
        let (sig, _) = fns
            .get(&twin)
            .unwrap_or_else(|| panic!("{twin} was not emitted"));
        assert!(
            sig.contains(&format!(", {params})")),
            "{twin}: the inputs must be `{params}`, got:\n{sig}"
        );
    }
    for name in NO_TWIN {
        assert!(fns.contains_key(*name), "{name} was not emitted");
        assert!(
            !fns.contains_key(&format!("{name}__inv")),
            "{name} must earn no twin"
        );
    }
    for (name, calls) in CALLS {
        let (_, got) = fns
            .get(*name)
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(*got, *calls, "{name}: twin calls in its body");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_emits_no_twin_and_no_twin_call() {
    let out = std::env::temp_dir().join("loft_callee_inputs_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_CALLEE_INPUTS", "1")]);
    assert!(
        !rust.contains("__inv("),
        "LOFT_NO_CALLEE_INPUTS=1 must emit no twin and call none"
    );
    let _ = std::fs::remove_file(&out);
}
