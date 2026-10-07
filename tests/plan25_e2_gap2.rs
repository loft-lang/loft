// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2 / gap 2 — coercing a `__nullable<S>` value to a dense `S` at a
//! VALUE boundary (a by-value `fn f(x: S)` argument fed a nullable vector
//! element).  Gate-on, the element is the synth `__nullable<S>` enum; the `Some`
//! payload is packed INDEPENDENTLY from dense `S` (the layout packer reorders
//! fields around the discriminant), so the coercion cannot reinterpret the
//! payload as dense `S` — it must copy FIELD BY FIELD (each field from its `Some`
//! offset to its dense offset) and deep-copy heap fields.  The runtime op
//! `OpNullableToDense` (→ `Stores::nullable_to_dense`) does this on both backends.
//!
//! The probe struct deliberately MIXES field kinds and widths (`integer`, `text`,
//! `integer not null`, `character`) so the dense and `Some` layouts diverge: a
//! shifted-reinterpret would read the reordered text/char fields as null/empty
//! (the silent corruption this op replaces).
//!
//! `LOFT_NO_CACHE=1` is mandatory — the warm program cache keys on source, not on
//! `LOFT_E2_SYNTH`, so a cached gate-off bundle would mask the rewrite.

use loft::file_access as fa;
use std::path::Path;
use std::process::Command;

fn run(args: &[&str], prog: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_E2_SYNTH", "1")
        .output()
        .expect("spawn loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn rustc_available() -> bool {
    Command::new("rustc").arg("--version").output().is_ok()
}

fn assert_both(prog: &Path, want: &str) {
    let (ok, out) = run(&["--interpret"], prog);
    assert!(ok, "interpret run failed; stdout={out:?}");
    assert!(
        out.contains(want),
        "interpret missing {want:?}; got {out:?}"
    );
    if rustc_available() {
        let (ok, out) = run(&["--native"], prog);
        assert!(ok, "native run failed; stdout={out:?}");
        assert!(out.contains(want), "native missing {want:?}; got {out:?}");
    }
}

fn probe(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_e2_gap2_probe");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, body).expect("write probe");
    path
}

const SRC_BY_VALUE: &str = "\
struct S { a: integer, b: text, c: integer not null, d: character }
fn allf(x: S) -> text { \"a={x.a} b={x.b} c={x.c} d={x.d}\" }
fn main() {
  v: vector<S> = [ S{a:1, b:\"hello\", c:7, d:'z'} ];
  print(\"{allf(v[0])}\\n\");
}
";

#[test]
fn nullable_element_to_dense_by_value_arg_field_copy() {
    // A `vector<S>` element (gate-on a `__nullable<S>` Some record) passed to a
    // by-value `fn allf(x: S)`. Each field must arrive correct — including the
    // reordered text `b` and char `d`, which a payload reinterpret reads as
    // null/empty. Both backends.
    assert_both(&probe("by_value", SRC_BY_VALUE), "a=1 b=hello c=7 d=z");
}

const SRC_DENSE_ASSIGN: &str = "\
struct S { a: integer, b: text, c: integer not null, d: character }
fn main() {
  v: vector<S> = [ S{a:1, b:\"hi\", c:7, d:'z'} ];
  dd: S = v[0];
  print(\"a={dd.a} b={dd.b} c={dd.c} d={dd.d}\\n\");
}
";

#[test]
fn nullable_element_to_dense_local_assign() {
    // A typed dense-`S` local assigned a `__nullable<S>` element (`dd: S = v[0]`).
    // The var-type-change check would reject "cannot change type from S to
    // __nullable<S>" (it fires on pass 1); routing through `convert` →
    // `OpNullableToDense` before `change_var` unwraps the value and retypes the
    // RHS dense. Native additionally needs the vector-read source HOISTED (it
    // emits a `stores.*` call) so the unwrap op's arg does not double-borrow.
    assert_both(&probe("dense_assign", SRC_DENSE_ASSIGN), "a=1 b=hi c=7 d=z");
}

const SRC_RETURN_VIA_LOCAL: &str = "\
struct M { hp: integer, depth: integer, name: text }
fn none() -> M { M { hp: 0, depth: 0, name: \"none\" } }
fn pick(t: vector<M>, i: integer) -> M {
  chosen = t[i] ?? none();
  chosen
}
fn main() {
  t: vector<M> = [ M{hp:8, depth:5, name:\"bat\"}, M{hp:3, depth:7, name:\"rat\"} ];
  a = pick(t, 0);
  print(\"hp={a.hp} depth={a.depth} name={a.name}\\n\");
}
";

#[test]
fn nullable_unwrap_returned_via_local_var() {
    // A `??`-unwrapped element assigned to a LOCAL then RETURNED as dense `M`
    // (with a heap `text` field).  Gate-on, `chosen` is `__nullable<M>` and the
    // return coerces it via `OpNullableToDense`.  On native the ref-return path
    // demoted that fresh-store unwrap tail to a discarded statement + `return
    // null` (caller derefs store 65535 → panic); `ref_return` now materialises a
    // `Var`-sourced unwrap tail into the return buffer.  Both backends.
    assert_both(
        &probe("return_via_local", SRC_RETURN_VIA_LOCAL),
        "hp=8 depth=5 name=bat",
    );
}

const SRC_RETURN_DIRECT_COALESCE: &str = "\
struct M { hp: integer, depth: integer, name: text }
fn none() -> M { M { hp: 0, depth: 0, name: \"none\" } }
fn pick(t: vector<M>, i: integer) -> M { t[i] ?? none() }
fn main() {
  t: vector<M> = [ M{hp:8, depth:5, name:\"bat\"} ];
  a = pick(t, 0);
  print(\"hp={a.hp} depth={a.depth} name={a.name}\\n\");
}
";

#[test]
fn nullable_unwrap_returned_directly_from_coalesce() {
    // The body-tail `??`-unwrap returned DIRECTLY (no local): `fn f() -> M { t[i]
    // ?? none() }`.  The tail is `OpNullableToDense(<?? ncc block>)` — a fresh-store
    // call whose source is a Block, not a Var.  On native the ref-return demoted it
    // to `return null` (caller derefs store 65535).  `ref_return` now materialises a
    // Var-/Block-/If-sourced unwrap tail into the return buffer (a plain `v[i]`
    // index tail stays the default direct return).  Both backends.
    assert_both(
        &probe("return_direct_coalesce", SRC_RETURN_DIRECT_COALESCE),
        "hp=8 depth=5 name=bat",
    );
}
