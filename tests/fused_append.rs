// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-m (loft#1426) — `v += [x]` on a plain vector of a scalar kind is ONE fused
//! `OpPush<Kind>` on both backends.
//!
//! The four-op lowering (`OpPreAllocVector · OpNewRecord · OpSet<Kind> · OpFinishRecord`) was
//! five runtime calls per element, each resolving the store and re-reading the headers the
//! previous one read.  The cell corpus (`bytecode-comparisons/V-m-fused-append-cells.loft`)
//! can only say the VALUES hold, because both lowerings answer the same numbers; this pins
//! the emission: the scalar append carries the fused op, a keyed collection keeps the
//! general path, a comprehension fuses too, and the switch (`LOFT_NO_FUSED_APPEND=1`)
//! restores the four ops — which is what makes this test red on the build before the fusion
//! and on one that lost it.  Read off `loft introspect`.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct K { k: integer }
struct Lay { xs: vector<float> }
fn c_scalar(n: integer) -> integer { v: vector<integer> = []; for i in 0..n { v += [i]; } len(v) }
fn c_keyed() -> integer { s: sorted<K[k]> = []; s += [K { k: 5 }]; s += [K { k: 1 }]; len(s) }
fn c_comprehension(n: integer) -> integer { lay = Lay { xs: [] }; lay.xs = [for i in 0..n { (i as float) * 0.5 }]; len(lay.xs) }
fn main() { println(\"{c_scalar(3)} {c_keyed()} {c_comprehension(4)}\"); }
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

/// The IR of one function, as `introspect` prints it (up to its byte-code section).
fn ir_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let start = dump
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no IR for {name}"));
    let rest = &dump[start..];
    let end = rest.find("\nbyte-code").unwrap_or(rest.len());
    &rest[..end]
}

fn write_probe(name: &str) -> PathBuf {
    let src = std::env::temp_dir().join(format!("loft_fused_append_{name}.loft"));
    std::fs::write(&src, PROBE).expect("write probe");
    src
}

#[test]
fn a_scalar_append_is_one_fused_push_and_a_keyed_insert_is_not() {
    let src = write_probe("on");
    let dump = introspect(&src, &[]);
    let scalar = ir_of(&dump, "n_c_scalar");
    assert!(
        scalar.contains("OpPushInt(") && !scalar.contains("OpNewRecord("),
        "the scalar append must be the fused push, with no record op left:\n{scalar}"
    );
    let keyed = ir_of(&dump, "n_c_keyed");
    assert!(
        !keyed.contains("OpPush") && keyed.contains("OpNewRecord("),
        "a keyed insert keeps the general path:\n{keyed}"
    );
    let comp = ir_of(&dump, "n_c_comprehension");
    assert!(
        comp.contains("OpPushFloat(") && !comp.contains("OpNewRecord("),
        "a scalar comprehension fuses too:\n{comp}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn the_switch_restores_the_four_op_form() {
    let src = write_probe("off");
    let dump = introspect(&src, &[("LOFT_NO_FUSED_APPEND", "1")]);
    let scalar = ir_of(&dump, "n_c_scalar");
    assert!(
        !scalar.contains("OpPush") && scalar.contains("OpNewRecord("),
        "with the fusion off the scalar append must keep OpNewRecord:\n{scalar}"
    );
    let _ = std::fs::remove_file(&src);
}
