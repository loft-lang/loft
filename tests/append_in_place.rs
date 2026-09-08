// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN157 § V-d (loft#1426) — the emitted shape of a vector-literal element that is a
//! buffer-returning call.
//!
//! The corpus guard (`tests/scripts/157-a-vector-element-is-built-in-place.loft`) can
//! only say the VALUES hold, because both lowerings answer the same numbers.  This pins
//! what changed: for a Route R callee the loop body claims the element, calls with the
//! element as the buffer, and tests the answer's store — and carries no lift temp, no
//! unconditional copy and no free.  For an NRVO callee (a return that names its buffer)
//! the body keeps the lift and the copy, which is the exclusion the matrix's A2/A4
//! demanded.  Read off `loft introspect`, the same instrument the design was written on.

use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct Pt { x: float = 0.0, y: float = 0.0 }
fn pt(a: float, b: float) -> Pt { Pt { x: a, y: b } }
fn nrvo(a: float) -> Pt { t = Pt { x: a, y: 0.0 }; t.y = a + 1.0; t }
fn in_place(n: integer) -> vector<Pt> { v: vector<Pt> = []; for i in 0..n { v += [pt(i as float, 1.0)]; } v }
fn kept(n: integer) -> vector<Pt> { v: vector<Pt> = []; for i in 0..n { v += [nrvo(i as float)]; } v }
fn main() { println(\"{len(in_place(3))} {len(kept(3))}\"); }
";

fn introspect(src: &std::path::Path) -> String {
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("introspect")
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The IR of one function, as `introspect` prints it (up to its byte-code section).
fn ir_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let start = dump
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no IR for {name}"));
    let rest = &dump[start..];
    let end = rest.find("\nbyte-code").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn a_route_r_callee_builds_the_element_in_place_and_an_nrvo_callee_keeps_the_copy() {
    let src = std::env::temp_dir().join("loft_append_in_place_probe.loft");
    std::fs::write(&src, PROBE).expect("write probe");
    let dump = introspect(&src);
    let _ = std::fs::remove_file(&src);

    let built = ir_of(&dump, "n_in_place");
    assert!(
        built.contains("OpNewRecord(v(") && built.contains("OpDistinctStore("),
        "the element must be claimed first and the answer's store tested:\n{built}"
    );
    assert!(
        built.contains("n_pt(") && built.contains(", _elm_1("),
        "the call must receive the ELEMENT as its buffer:\n{built}"
    );
    assert!(
        !built.contains("__lift_"),
        "no lift temp may stand between the call and the element:\n{built}"
    );

    let kept = ir_of(&dump, "n_kept");
    assert!(
        kept.contains("__lift_") && !kept.contains("OpDistinctStore("),
        "an NRVO callee (its return names its buffer) must keep today's lift + copy:\n{kept}"
    );
}
