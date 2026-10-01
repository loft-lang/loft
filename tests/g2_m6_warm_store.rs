// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 G2/M6 — warm-cache store-backed codegen, end-to-end.
//!
//! With the program cache on and `LOFT_CODEGEN_STORE`, a warm cache hit loads the
//! bundle as a *skeleton* (def table only — `open_program_store` /
//! `read_data_skeleton`) and codegen reads each function body straight from the
//! mmap'd store via `def_body_node`, skipping `read_data`'s body reconstruction.
//! Output must be byte-identical to native.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `script`; `mode` selects env: native / cold-cache / warm-M6.
fn run(script: &std::path::Path, cache: Option<&std::path::Path>, m6: bool) -> (bool, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(script)
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_CODEGEN_STORE");
    if let Some(dir) = cache {
        cmd.env_remove("LOFT_NO_CACHE").env("XDG_CACHE_HOME", dir);
        if m6 {
            cmd.env("LOFT_CODEGEN_STORE", "1");
        }
    }
    let out = cmd.output().expect("invoke loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn warm_m6_store_matches_native() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_m6_{pid}.loft"));
    std::fs::write(
        &script,
        "struct Point { x: integer, y: integer }\n\
         fn fib(n: integer) -> integer { if n < 2 { n } else { fib(n-1) + fib(n-2) } }\n\
         fn dist2(p: Point) -> integer { p.x*p.x + p.y*p.y }\n\
         fn main() {\n\
         \x20 s=\"\"; for i in 0..12 { s = s + \"{fib(i)} \"; } print(\"{s}\\n\");\n\
         \x20 for p in [Point{x:3,y:4}, Point{x:5,y:12}] { print(\"d2={dist2(p)}\\n\"); }\n\
         \x20 t=(7,11); print(\"t={t.0},{t.1}\\n\");\n\
         }\n",
    )
    .expect("write script");
    let cache = tmp.join(format!("loft_m6_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache);

    let (ok_native, out_native) = run(&script, None, false);
    assert!(ok_native, "native run failed: {out_native}");
    assert!(
        out_native.contains("0 1 1 2 3 5 8 13 21 34"),
        "fib: {out_native}"
    );

    // Cold run primes the bundle.
    let (ok_cold, _) = run(&script, Some(&cache), false);
    assert!(ok_cold, "cold cache run failed");

    // Warm M6: skeleton load + codegen from the mmap'd store.
    let (ok_m6, out_m6) = run(&script, Some(&cache), true);
    assert!(ok_m6, "warm M6 run failed: {out_m6}");
    assert_eq!(
        out_native, out_m6,
        "warm-M6 store codegen diverged from native"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache);
}

/// A MULTI-source program — `use importlib::*` beside its own definitions — survives
/// the warm load: the bundle carries the import tables, so the loaded `Data` reaches
/// every wildcard-imported name exactly as the fresh parse did (loft#1359).
#[test]
fn warm_store_keeps_wildcard_import_bindings() {
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("loft_m6_import_{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("importlib.loft"),
        "pub struct Point { x: integer, y: integer }\n\
         pub fn add(a: integer, b: integer) -> integer { a + b }\n\
         pub fn mul(a: integer, b: integer) -> integer { a * b }\n",
    )
    .expect("write lib");
    let script = dir.join("main.loft");
    std::fs::write(
        &script,
        "use importlib::*;\n\
         fn main() {\n\
         \x20 p = Point { x: 3, y: 4 };\n\
         \x20 print(\"{add(p.x, p.y)} {mul(p.x, p.y)}\\n\");\n\
         }\n",
    )
    .expect("write script");
    let cache = dir.join("cache");

    let (ok_native, out_native) = run(&script, None, false);
    assert!(ok_native, "native run failed: {out_native}");
    assert_eq!(out_native.trim(), "7 12", "the fresh parse: {out_native}");

    let (ok_cold, out_cold) = run(&script, Some(&cache), false);
    assert!(ok_cold, "cold cache run failed: {out_cold}");
    let (ok_warm, out_warm) = run(&script, Some(&cache), true);
    assert!(ok_warm, "warm run failed: {out_warm}");
    assert_eq!(out_native, out_warm, "the warm load lost an imported name");

    let _ = std::fs::remove_dir_all(&dir);
}
