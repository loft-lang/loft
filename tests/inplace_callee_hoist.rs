// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-l (loft#1426) — a loop that calls an IN-PLACE-ONLY writer keeps its hoisted
//! vector headers.
//!
//! The cell corpus (`bytecode-comparisons/V-l-in-place-callee-cells.loft`) can only say the
//! VALUES hold, because the interpreter has no hoist and native answers the same numbers
//! either way.  This pins the emission: a loop calling a setter whose only store write is a
//! scalar set through an element address derives its vector header once (`vec_header(` in
//! the emitted Rust), a loop calling a callee that GROWS the vector it reads does not, and
//! the switch (`LOFT_NO_INPLACE_CALLEE_HOIST=1`) restores the unhoisted form — which is
//! what makes this test red on the build before the allowance and on one that lost it.
//! Read off `loft introspect`, which carries the native Rust beside the IR.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct Cv { data: vector<integer>, count: integer }
fn setp(cv: Cv, i: integer, v: integer) { d = &cv.data; if i >= 0 && i < len(d) { d[i] = v; } }
fn grow(cv: Cv, v: integer) { cv.data += [v]; }
fn c_setter() -> integer {
  src: vector<integer> = [3, 5, 7, 9];
  cv = Cv { data: [0, 0, 0, 0], count: 0 };
  for i in 0..len(src) { setp(cv, i, src[i]? * 2); }
  cv.data[3]?
}
fn c_grower() -> integer {
  cv = Cv { data: [1, 2, 3], count: 0 };
  s = 0;
  for i in 0..3 { s += cv.data[i]?; grow(cv, 10 + i); }
  s
}
fn main() { println(\"{c_setter()} {c_grower()}\"); }
";

fn introspect(src: &std::path::Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("introspect").arg(src).env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The emitted Rust of one function, as `introspect` prints it after the IR sections.
fn rust_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let key = format!("fn {name}(cell:");
    let start = dump
        .find(&key)
        .unwrap_or_else(|| panic!("no emitted Rust for {name}"));
    let rest = &dump[start..];
    let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

fn headers(rust: &str) -> usize {
    rust.matches("vec_header(").count()
}

fn write_probe(name: &str) -> PathBuf {
    let src = std::env::temp_dir().join(format!("loft_inplace_callee_{name}.loft"));
    std::fs::write(&src, PROBE).expect("write probe");
    src
}

#[test]
fn a_loop_calling_an_in_place_setter_hoists_and_a_grower_blocks() {
    let src = write_probe("on");
    let dump = introspect(&src, &[]);
    let setter = rust_of(&dump, "n_c_setter");
    assert!(
        headers(setter) >= 1,
        "the loop calling `setp` must derive its header once:\n{setter}"
    );
    let grower = rust_of(&dump, "n_c_grower");
    assert_eq!(
        headers(grower),
        0,
        "the loop calling `grow` must keep the per-element form:\n{grower}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn the_switch_restores_the_unhoisted_form() {
    let src = write_probe("off");
    let dump = introspect(&src, &[("LOFT_NO_INPLACE_CALLEE_HOIST", "1")]);
    let setter = rust_of(&dump, "n_c_setter");
    assert_eq!(
        headers(setter),
        0,
        "with the allowance off the setter-calling loop must not hoist:\n{setter}"
    );
    let _ = std::fs::remove_file(&src);
}
