// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 G2/M5 — store-backed codegen equivalence proof.
//!
//! With `LOFT_CODEGEN_STORE` set, `def_code` materialises each function body
//! into a store and lowers it through `IrNode::Store` instead of the native
//! `&Value` graph — `generate_inner` reads the body via the handle (store) and
//! the Definition table via the native `Data`.  The emitted bytecode, and thus
//! the program's behaviour, must be identical to the native path.  This test
//! runs a program exercising every major lowering arm both ways and asserts the
//! output matches.

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn run(script: &std::path::Path, store_backed: bool) -> (bool, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret").arg(script);
    if store_backed {
        cmd.env("LOFT_CODEGEN_STORE", "1");
    } else {
        cmd.env_remove("LOFT_CODEGEN_STORE");
    }
    let out = cmd.output().expect("invoke loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn store_backed_codegen_matches_native() {
    let pid = std::process::id();
    let script = std::env::temp_dir().join(format!("loft_m5_{pid}.loft"));
    // Exercises: recursion + if-expr, struct construct + field read, for-loops
    // over ranges and vectors, text interpolation (Call + text dest), Set
    // reassignment, tuple get/put, vector indexing — i.e. most generate_inner arms.
    fa::write(
        &script,
        r#"
struct Point { x: integer, y: integer }
fn fib(n: integer) -> integer { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn dist2(p: Point) -> integer { p.x * p.x + p.y * p.y }
fn main() {
  s = "";
  for i in 0..10 { s = s + "{fib(i)} "; }
  print("{s}\n");
  pts = [Point{x: 3, y: 4}, Point{x: 5, y: 12}];
  for p in pts { print("d2={dist2(p)}\n"); }
  v = [3, 1, 4, 1, 5, 9];
  total = 0;
  for x in v { total = total + x; }
  print("total={total} first={v[0]} len={v.len()}\n");
  t = (7, 11);
  print("tuple={t.0},{t.1}\n");
}
"#,
    )
    .expect("write script");

    let (ok_native, out_native) = run(&script, false);
    let (ok_store, out_store) = run(&script, true);
    assert!(ok_native, "native run failed: {out_native}");
    assert!(ok_store, "store-backed run failed: {out_store}");
    assert!(
        out_native.contains("0 1 1 2 3 5 8 13 21 34"),
        "fib output: {out_native}"
    );
    assert_eq!(
        out_native, out_store,
        "store-backed codegen diverged from native"
    );

    let _ = fa::remove_file(&script);
}
