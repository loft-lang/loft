// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-ad (loft#1426) — a null-discharge DEFAULT BUFFER does not block the hoist
//! (`@FR-R-InPlace`, the hidden-buffer allowance): `e = tbl[i]?` on a vector of all-scalar
//! records mints an absent element into a hidden `__ref_p2_N` buffer, and the allocation
//! into that buffer is admitted at the header gate, so the loop around it keeps its headers.
//!
//! The cell corpus (`bytecode-comparisons/V-ad-null-buffer-cells.loft`) can only say the
//! VALUES hold.  This pins the EMISSION: how many headers each cell's function derives, that a
//! heap-owning record still derives none, and the switch (`LOFT_NO_NULL_BUFFER_HOIST=1`),
//! which is what makes it red on the build before the unit and on one that lost it.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-ad-null-buffer-cells.loft";

/// `(cell function, vector headers derived, push headers derived)` with the admission on.
const HEADERS: &[(&str, usize, usize)] = &[
    ("n_d1", 2, 0), // t and v
    ("n_d2", 2, 0), // the default arm taken changes nothing at generation time
    ("n_d3", 2, 1), // a heap-owning record: @PLN158 P1's lift means the discharge buffer's
    //                  mint no longer blocks the loop whatever the record holds, so h and v
    //                  hold their headers; the one push header is the literal
    //                  `h = [Hv {…}, Hv {…}]`'s own group header (`@FR-R-GroupPush`).
    //                  This row read (0, 1) for a day on the quality branch, where P1 was
    //                  REVERTED because it read a stale header on `1575`'s `c3` — a local
    //                  rebound from a call in a loop, `null(oob)` for `1` on native.  P1 is
    //                  back since the join of 2026-09-23 and that cell passes on both
    //                  backends again, because the same line brought `@PLN158`'s later fix:
    //                  a hoist's root is invariant only while what it LINKS to is.  The pair
    //                  is re-measured here rather than carried from either side.
    ("n_d4", 3, 0),  // t and v in the loop, v again in the summing loop
    ("n_d5", 1, 1),  // v read, out pushed; t is a call result and leaves the read list
    ("n_d6", 2, 0),  // derived by the outer loop once
    ("n_d7", 2, 0),  // two sites, still t and v
    ("n_d8", 1, 0),  // t serves the view and the scalar read
    ("n_d9", 1, 0),  // t
    ("n_d10", 2, 0), // t and v
    // d11–d13: the LOOP stays blocked (no vector header); the one push header each is the
    // seeding literal's own group header (`@FR-R-GroupPush`, 2026-09-18 — a heap-owning
    // element takes it too), bound before the loop and closed at the literal's last finish.
    ("n_d11", 0, 1), // a heap-owning record: blocked
    ("n_d12", 0, 1), // the alias shape over a record with a default vector: blocked
    ("n_d13", 1, 1), // the alias falsifier proper: heap-owning, no default — with P1 the
                     //                  discharge no longer blocks the loop, and the LINK clause keeps
                     //                  `g.tags` out (g links to e, which the body rebinds): the one header
                     //                  is `h`'s, and the values test reads 7
];

/// The cells whose loop is blocked without the admission: under the switch none derives a
/// header (d4 keeps its summing loop's one, d5 its push header — neither discharges).
const BLOCKED_OFF: &[(&str, usize, usize)] = &[
    ("n_d1", 0, 0),
    ("n_d2", 0, 0),
    ("n_d4", 1, 0),
    ("n_d5", 0, 0),
    ("n_d6", 0, 0),
    ("n_d7", 0, 0),
    ("n_d8", 0, 0),
    ("n_d9", 0, 0),
    ("n_d10", 0, 0),
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

/// Per emitted function: `(vector headers, push headers)` BOUND in its body — the
/// prelude's `let __vh_N` / `let mut __ph_N`.  A re-derivation into an existing binding
/// (§ V-am re-derives a push header after its reserve, `__ph_N = vector::push_header(…)`)
/// is the same header the loop holds, not another one.
fn headers(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
            continue;
        }
        if line.contains("let __vh_") && line.contains("vector::vec_header(&(") {
            map.entry(current.clone()).or_default().0 += 1;
        }
        if line.contains("let mut __ph_") && line.contains("vector::push_header(&(") {
            map.entry(current.clone()).or_default().1 += 1;
        }
    }
    map
}

#[test]
fn a_discharge_buffer_leaves_the_loops_headers_in_place() {
    let out = std::env::temp_dir().join("loft_null_buffer_hoist_on.rs");
    let fns = headers(&emit(&out, &[]));
    for (name, vec, push) in HEADERS {
        let got = fns
            .get(*name)
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            *got,
            (*vec, *push),
            "{name}: (vector headers, push headers)"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_blocks_every_discharging_loop_again() {
    let out = std::env::temp_dir().join("loft_null_buffer_hoist_off.rs");
    let fns = headers(&emit(&out, &[("LOFT_NO_NULL_BUFFER_HOIST", "1")]));
    for (name, vec, push) in BLOCKED_OFF {
        let got = fns
            .get(*name)
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            *got,
            (*vec, *push),
            "{name} under LOFT_NO_NULL_BUFFER_HOIST=1: (vector headers, push headers)"
        );
    }
    let _ = std::fs::remove_file(&out);
}
