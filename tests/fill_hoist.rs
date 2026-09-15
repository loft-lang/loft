// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-ae (loft#1426) — the FILL idiom (`@FR-R-Fill`): `for i in lo..hi { v[base + i]
//! = c }` over a held header, `base` and `c` invariant, emits ONE guarded slice fill with the
//! per-element loop as its fallback.
//!
//! The cell corpus (`bytecode-comparisons/V-ae-fill-cells.loft`) can only say the VALUES hold —
//! the fill and the loop write the same elements.  This pins the EMISSION: which loops take the
//! fill, which keep the per-element form, that every fill carries its fallback and its counter
//! tail, and the switch (`LOFT_NO_FILL_HOIST=1`), which is what makes it red on the build
//! before the unit and on one that lost it.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-ae-fill-cells.loft";

/// `(function, fills emitted in its body)` with the rewrite on.
const FILLS: &[(&str, usize)] = &[
    ("n_hline", 1),      // pil_hline's shape; f1, f2, f11, f12 call it
    ("n_hline__inv", 1), // its § V-p twin fills through the handed header
    ("n_f1", 0),         // the fill is inside hline
    ("n_f3", 1),         // a range starting below 0: the fill declines at run time
    ("n_f4", 1),         // an empty range: declines at run time
    ("n_f5", 1),         // an exclusive range
    ("n_f6", 0),         // a non-contiguous index
    ("n_f7", 0),         // the value depends on the loop variable
    ("n_f8", 0),         // the value reads the vector
    ("n_f9", 0),         // two statements in the body
    ("n_f10", 1),        // a float vector through a record path
    ("n_f13", 0),        // a `??` block as the value
    ("n_f14", 1),        // an empty vector: declines at run time
    ("n_f15", 1),        // the large range
    ("n_f16", 0),        // two statements in the body
];

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(&src)
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

/// Per emitted function: `(fills, fallback guards, counter tails)`.
fn fills(rust: &str) -> HashMap<String, (usize, usize, usize)> {
    let mut map: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
            continue;
        }
        let e = map.entry(current.clone()).or_default();
        if line.contains("stores.fill_hoisted::<") {
            e.0 += 1;
        }
        if line.trim_start().starts_with("if !__fill_") {
            e.1 += 1;
        }
        if line.contains("} else { var_") && line.contains("__index = ") {
            e.2 += 1;
        }
    }
    map
}

#[test]
fn each_filling_loop_takes_the_fill_with_its_fallback_and_tail() {
    let out = std::env::temp_dir().join("loft_fill_hoist_on.rs");
    let fns = fills(&emit(&out, &[]));
    for (name, n) in FILLS {
        let got = fns
            .get(*name)
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(*got, (*n, *n, *n), "{name}: (fills, guards, tails)");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_emits_no_fill() {
    let out = std::env::temp_dir().join("loft_fill_hoist_off.rs");
    let rust = emit(&out, &[("LOFT_NO_FILL_HOIST", "1")]);
    assert!(
        !rust.contains("fill_hoisted"),
        "LOFT_NO_FILL_HOIST=1 must emit no fill"
    );
    let _ = std::fs::remove_file(&out);
}
