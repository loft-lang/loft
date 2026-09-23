// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-Stride` — a vector literal reserves at its ELEMENT's width.
//!
//! `OpPreAllocVector(v, n, width)` asked `database.size(known_type)`, which answers 8 for the
//! integer base whatever the declaration said, so a `vector<u8>` literal reserved eight times
//! the bytes its own reads then walked at stride 1 (`OpGetVectorNullable(b, 1, i)`).  The
//! VALUES cannot see it — the append path grows the store as it needs to and every element
//! reads back correctly, which is why this is an emission pin and not a script.  It is the
//! fifth site to ask this question with its own decoder; loft#1420 closed the other four by
//! routing them through `Parser::element_store_size`, and this routes the reservation there.
//!
//! Red on a build that reserves 8 for a narrow element, and on one that loses the nested
//! element's handle row.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct P { a: integer, s: text }
fn main() {
  b: vector<u8> = [9, 7, 250];
  h: vector<i16> = [-2, 300];
  w: vector<u32> = [1, 4000000000];
  c: vector<integer> = [9, 7];
  p: vector<P> = [P { a: 1, s: \"x\" }];
  nv: vector<vector<integer>> = [[1, 2], [3]];
  println(\"{len(b)}{len(h)}{len(w)}{len(c)}{len(p)}{len(nv)}\");
}
";

fn introspect(src: &std::path::Path) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect").arg(src).env("LOFT_TIMEOUT", "120");
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every `OpPreAllocVector` in the emission, as `(count, width)` in source order.
fn reservations(ir: &str) -> Vec<(i32, i32)> {
    ir.lines()
        .filter_map(|l| l.split_once("OpPreAllocVector("))
        .filter_map(|(_, rest)| {
            // `OpPreAllocVector(b(1), 3i32, 1i32);` — the leading integer of each argument.
            let number = |a: &str| -> Option<i32> {
                let a = a.trim();
                let end = a
                    .char_indices()
                    .find(|(i, c)| !(c.is_ascii_digit() || (*i == 0 && *c == '-')))
                    .map_or(a.len(), |(i, _)| i);
                a[..end].parse().ok()
            };
            let args: Vec<&str> = rest.split(',').collect();
            Some((number(args.get(1)?)?, number(args.get(2)?)?))
        })
        .collect()
}

#[test]
fn a_vector_literal_reserves_at_its_elements_width() {
    let src = std::env::temp_dir().join(format!("loft_prealloc_{}.loft", std::process::id()));
    std::fs::write(&src, PROBE).expect("write probe");
    let ir = introspect(&src);
    let _ = std::fs::remove_file(&src);
    let got = reservations(&ir);
    assert!(
        !got.is_empty(),
        "no OpPreAllocVector in the emission — the probe stopped reaching the site it pins:\n{ir}"
    );
    // b: 3 × 1, h: 2 × 2, w: 2 × 4, c: 2 × 8, p: 1 × record, nv: 2 × handle row.
    // The narrow three are the point; the others are the controls that must not move with
    // them.  A record's and a nested vector's widths are layout figures, so they are asserted
    // as "not the integer default" rather than as numbers this test would have to re-derive.
    assert_eq!(
        got.iter().take(4).copied().collect::<Vec<_>>(),
        vec![(3, 1), (2, 2), (2, 4), (2, 8)],
        "u8/i16/u32 reserve at 1/2/4 and a plain integer at 8; got {got:?}"
    );
    assert!(
        got.len() >= 6 && got[4].1 > 0 && got[5].1 > 0,
        "the record and nested-vector literals still reserve: {got:?}"
    );
}
