// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-g (loft#1426) — the emitted shape of a read-only record local bound from a
//! call whose return BORROWS a value-const argument.
//!
//! The corpus guard (`tests/scripts/157-a-read-only-record-local-keeps-the-view.loft`) can
//! only say the VALUES hold, because both lowerings answer the same numbers.  This pins
//! what changed: the local keeps its dep (a view), is bound by the plain delivery — no
//! `OpBindOrCopy`, no `OpCopyRecord` — and is released by store identity against the
//! argument at scope exit, so only the callee's per-execution minted store is ever freed
//! through it.  A written local and a rebound base keep today's copy, and so does the
//! switch (`LOFT_NO_VIEW_ELISION=1`), which is what makes this test red on the build before
//! the elision and on a build that lost it.  Read off `loft introspect`, the instrument the
//! design was written on.
use std::path::PathBuf;
use std::process::Command;

const PROBE: &str = "\
struct Pt { ptx: float, pty: float }
fn ctrl(pts: const vector<Pt>, i: integer) -> Pt { ci = i; if ci < 0 { ci = 0; } pts[ci]? }
fn c1(v: const vector<Pt>) -> float { a = ctrl(v, 1); a.ptx + a.pty }
fn c5(v: const vector<Pt>) -> float { a = ctrl(v, 1); a.ptx = 9.0; a.ptx + v[1]?.ptx }
fn c11(v: const vector<Pt>, other: const vector<Pt>) -> float { a = ctrl(v, 1); v = other; a.ptx + v[0]?.ptx }
fn main() {
  v: vector<Pt> = [Pt{ptx: 1.0, pty: 2.0}, Pt{ptx: 3.0, pty: 5.0}];
  w: vector<Pt> = [Pt{ptx: 20.0, pty: 30.0}];
  println(\"{c1(v)} {c5(v)} {c11(v, w)}\");
}
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

/// The byte-code of one function, as `introspect` prints it.
fn bytecode_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let key = format!(":{name}(");
    let start = dump
        .find("byte-code for ")
        .and_then(|i| dump[i..].find(&key).map(|j| i + j))
        .unwrap_or_else(|| panic!("no byte-code for {name}"));
    let rest = &dump[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    &rest[..end]
}

/// One file per test: the three run in parallel threads, and a shared path would let one
/// test's cleanup remove the probe another is about to introspect.
fn write_probe(name: &str) -> PathBuf {
    let src = std::env::temp_dir().join(format!("loft_view_elision_{name}.loft"));
    std::fs::write(&src, PROBE).expect("write probe");
    src
}

#[test]
fn a_read_only_local_keeps_the_view_and_is_freed_by_identity() {
    let src = write_probe("view");
    let dump = introspect(&src, &[]);
    let c1 = ir_of(&dump, "n_c1");
    assert!(
        c1.contains("a(1):ref(Pt)[\"v\"] = n_ctrl("),
        "the local must keep its dep on the argument (a view):\n{c1}"
    );
    assert!(
        c1.contains("OpFreeRefIfDistinct(a(1), v(0))"),
        "the scope exit must release the minted arm by identity against the argument:\n{c1}"
    );
    assert!(
        !c1.contains("OpFreeRef(a(1))")
            && !c1.contains("OpCopyRecord(")
            && !c1.contains("OpBindOrCopy"),
        "no copy and no unconditional free may reach an elided local:\n{c1}"
    );
    let bc = bytecode_of(&dump, "n_c1");
    assert!(
        !bc.contains("BindOrCopy") && !bc.contains("CopyRecord"),
        "the interpreter must bind the view with a plain PutRef:\n{bc}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn a_written_local_and_a_rebound_base_keep_the_copy() {
    let src = write_probe("copy");
    let dump = introspect(&src, &[]);
    for name in ["n_c5", "n_c11"] {
        let ir = ir_of(&dump, name);
        assert!(
            ir.contains("OpFreeRef(a(1))") && !ir.contains("OpFreeRefIfDistinct(a(1)"),
            "{name} must own a copy — its local is written, or its base is rebound:\n{ir}"
        );
    }
    let _ = std::fs::remove_file(&src);
}

#[test]
fn the_switch_restores_the_copy() {
    let src = write_probe("switch");
    let dump = introspect(&src, &[("LOFT_NO_VIEW_ELISION", "1")]);
    let c1 = ir_of(&dump, "n_c1");
    assert!(
        c1.contains("OpFreeRef(a(1))") && !c1.contains("OpFreeRefIfDistinct(a(1)"),
        "under LOFT_NO_VIEW_ELISION=1 the local must own a copy again:\n{c1}"
    );
    let _ = std::fs::remove_file(&src);
}
