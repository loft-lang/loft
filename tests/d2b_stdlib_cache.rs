// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 arc D / D2b — the opt-in stdlib startup cache, end-to-end.
//!
//! Runs a real program three ways through the `loft` binary: cache off
//! (default), cache on cold (parses + writes the bundle), cache on warm (mmaps
//! the bundle, skipping the `default/` parse).  All three must produce
//! byte-identical output, and the cold run must write the `.store` bundle.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the binary on `script`, optionally with the stdlib cache enabled at
/// `cache_dir` (`XDG_CACHE_HOME`).  Returns `(success, stdout)`.
fn run(script: &std::path::Path, cache_dir: Option<&std::path::Path>) -> (bool, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(script)
        .current_dir(workspace_root());
    if let Some(dir) = cache_dir {
        cmd.env("LOFT_STDLIB_CACHE", "1").env("XDG_CACHE_HOME", dir);
    } else {
        // Ensure the parent env doesn't leak the cache into the "off" run.
        cmd.env_remove("LOFT_STDLIB_CACHE");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn stdlib_cache_off_cold_warm_match() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_d2b_{pid}.loft"));
    std::fs::write(
        &script,
        "fn main() {\n  a = 3 + 4;\n  print(\"sum={a}\\n\");\n  \
         v = [10, 20, 30];\n  print(\"len={len(v)}\\n\");\n  \
         print(\"first={v[0]}\\n\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_d2b_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // 1. Cache OFF (default behaviour).
    let (ok_off, out_off) = run(&script, None);
    assert!(ok_off, "cache-off run failed: {out_off}");
    assert!(out_off.contains("sum=7"), "unexpected output: {out_off}");

    // 2. Cache ON, cold — parses default/ then writes the bundle.
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold cache run failed: {out_cold}");
    let store = std::fs::read_dir(cache_dir.join("loft"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("store"));
    assert!(store.is_some(), "cold run did not write a .store bundle");

    // 3. Cache ON, warm — mmaps the bundle, skips the parse.
    let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
    assert!(ok_warm, "warm cache run failed: {out_warm}");

    // All three byte-identical.
    assert_eq!(
        out_off, out_cold,
        "cold-cache output differs from cache-off"
    );
    assert_eq!(
        out_off, out_warm,
        "warm-cache output differs from cache-off"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// A stdlib GENERIC instantiated from a warm bundle runs as it does cold.
///
/// The program above instantiates no generic, so it could not see this: a function
/// reconstructed from the bundle is marked as already scoped, a generic template included,
/// and an instance used to inherit that mark — so the scope pass skipped every instance of a
/// stdlib generic on a warm start, and `tree_walk<Crate>` in `tests/docs/25-generics.loft`
/// was an internal compiler error (`var_pos underflow … '__ref_1'`) where a cold start ran it.
#[test]
fn a_stdlib_generic_instance_runs_the_same_warm() {
    let pid = std::process::id();
    let script = workspace_root().join("tests/docs/25-generics.loft");
    let cache_dir = std::env::temp_dir().join(format!("loft_d2b_generic_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);
    let (ok_off, out_off) = run(&script, None);
    assert!(ok_off, "cache-off run failed: {out_off}");
    let (ok_cold, out_cold) = run(&script, Some(&cache_dir));
    assert!(ok_cold, "cold cache run failed: {out_cold}");
    let (ok_warm, out_warm) = run(&script, Some(&cache_dir));
    assert!(ok_warm, "warm cache run failed: {out_warm}");
    assert_eq!(
        out_off, out_cold,
        "cold-cache output differs from cache-off"
    );
    assert_eq!(
        out_off, out_warm,
        "warm-cache output differs from cache-off"
    );
    let _ = std::fs::remove_dir_all(&cache_dir);
}

/// @PLN11 arc E — a corrupt / non-store bundle at the cache path must be a
/// clean cache miss (graceful reparse), never a crash.
#[test]
fn corrupt_bundle_falls_back_to_parse() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let script = tmp.join(format!("loft_d2b_corrupt_{pid}.loft"));
    std::fs::write(
        &script,
        "fn main() {\n  x = 1 + 1;\n  print(\"ok={x}\\n\");\n}\n",
    )
    .expect("write script");
    let cache_dir = tmp.join(format!("loft_d2b_corrupt_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);

    // Cold run writes a valid bundle.
    let (ok, out) = run(&script, Some(&cache_dir));
    assert!(ok, "cold run failed: {out}");
    let store = std::fs::read_dir(cache_dir.join("loft"))
        .expect("cache dir")
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("store"))
        .expect("bundle written")
        .path();

    // Overwrite the bundle with garbage (a partial / foreign file).
    std::fs::write(&store, b"definitely not a valid loft store image").expect("corrupt");

    // The next cache-on run must recover by reparsing — exit 0, correct output.
    let (ok2, out2) = run(&script, Some(&cache_dir));
    assert!(
        ok2,
        "corrupt-bundle run should recover by reparsing; got: {out2}"
    );
    assert!(out2.contains("ok=2"), "unexpected output: {out2}");

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}
