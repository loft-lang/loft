// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-l (loft#1426) — a loop that calls an IN-PLACE-ONLY writer keeps its hoisted
//! vector headers.
//!
//! The cell corpus (`bytecode-comparisons/V-l-in-place-callee-cells.loft`) can only say the
//! VALUES hold, because the interpreter has no hoist and native answers the same numbers
//! either way.  This pins the emission: a loop calling a setter whose only store write is a
//! scalar set through an element address derives its vector header once (`loft_vh_of(` in
//! the emitted C), a loop calling a callee that GROWS the vector it reads does not, and
//! the switch (`LOFT_NO_INPLACE_CALLEE_HOIST=1`) restores the unhoisted form — which is
//! what makes this test red on the build before the allowance and on one that lost it.
//! Read off `loft --native-emit`, the C the program compiles to; the verdict is the shared
//! `opt::writes` one, so the C backend takes it as the rustc writer did.
use loft::file_access as fa;
use std::path::PathBuf;

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

/// The C `loft --native-emit` writes for `src`.
fn emit_c(src: &std::path::Path, tag: &str, env: &[(&str, &str)]) -> String {
    let out = std::env::temp_dir().join(format!(
        "loft_inplace_callee_{tag}_{}.c",
        std::process::id()
    ));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_INPLACE_CALLEE_HOIST")
        .env_remove("LOFT_NO_APART_HEADERS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft --native-emit");
    let c = fa::read_to_string(&out).unwrap_or_else(|_| {
        panic!(
            "no C emitted (exit {:?}): {}",
            res.status,
            String::from_utf8_lossy(&res.stderr)
        )
    });
    let _ = fa::remove_file(&out);
    c
}

/// Every definition the C backend emits for loft function `name`: the function itself and
/// the parts it is lowered into (`<name>__fr`, `<name>__part<k>`), concatenated.
fn c_of(c: &str, name: &str) -> String {
    let mut bodies = String::new();
    for (at, _) in c.match_indices(&format!(" {name}")) {
        let rest = &c[at + 1 + name.len()..];
        if !(rest.starts_with('(') || rest.starts_with("__")) {
            continue;
        }
        let line_start = c[..at].rfind('\n').map_or(0, |i| i + 1);
        let line_end = c[at..].find('\n').map_or(c.len(), |i| at + i);
        let line = &c[line_start..line_end];
        if !line.starts_with("static ") || !line.ends_with(") {") {
            continue;
        }
        let end = c[line_start..]
            .find("\n}\n")
            .map_or(c.len(), |i| line_start + i + 3);
        bodies.push_str(&c[line_start..end]);
    }
    assert!(!bodies.is_empty(), "no emitted C for {name}");
    bodies
}

fn headers(c: &str) -> usize {
    c.matches("loft_vh_of(").count()
}

/// The headers derived for a vector reached through a record field — `cv.data`, the vector
/// the setter writes.  `src`, which the loop only reads, keeps its header through
/// `@FR-R-Header`'s apart clause whether the allowance is on or not, so the plain count
/// alone would not see the allowance go.
fn field_headers(c: &str) -> usize {
    c.matches("loft_vh_of(cx, loft_field(").count()
}

fn write_probe(name: &str) -> PathBuf {
    let src = std::env::temp_dir().join(format!("loft_inplace_callee_{name}.loft"));
    fa::write(&src, PROBE).expect("write probe");
    src
}

#[test]
fn a_loop_calling_an_in_place_setter_hoists_and_a_grower_blocks() {
    let src = write_probe("on");
    let c = emit_c(&src, "on", &[]);
    let setter = c_of(&c, "n_c_setter");
    assert_eq!(
        field_headers(&setter),
        1,
        "the loop calling `setp` must derive the header of `cv.data` once:\n{setter}"
    );
    let grower = c_of(&c, "n_c_grower");
    assert_eq!(
        headers(&grower),
        0,
        "the loop calling `grow` must keep the per-element form:\n{grower}"
    );
    let _ = fa::remove_file(&src);
}

#[test]
fn the_switch_restores_the_unhoisted_form() {
    let src = write_probe("off");
    // `@FR-R-Header`'s apart clause off too: it keeps the header of a vector the declined
    // loop only reads, which is not what this switch restores.
    let c = emit_c(
        &src,
        "off",
        &[
            ("LOFT_NO_INPLACE_CALLEE_HOIST", "1"),
            ("LOFT_NO_APART_HEADERS", "1"),
        ],
    );
    let setter = c_of(&c, "n_c_setter");
    assert_eq!(
        headers(&setter),
        0,
        "with the allowance off the setter-calling loop must not hoist:\n{setter}"
    );
    let _ = fa::remove_file(&src);
}
