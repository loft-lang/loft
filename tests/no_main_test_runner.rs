// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Running a file with no `fn main()` invokes the auto fallback, which runs every
// zero-param void user function (the #358 contract — see
// `tests/arc_e_program_cache.rs`). It must:
//   1. run a plain zero-param function (e.g. `helper()`), NOT only `test_*` — a
//      name gate here would silently break #358, and
//   2. never PANIC — in particular it must not collect a used library's
//      `#native` zero-param host imports (e.g. graphics' `gl_swap_buffers`) and
//      try to execute them, which hit a `def(u32::MAX)` "Unknown definition"
//      panic in `State::execute_argv`. `#native` fns are excluded by
//      `def.native.is_empty()` (no loft body to run).
// A `#native`-declared zero-param void function stands in for that host-import
// shape without needing a real library; the `execute_argv` guard makes even a
// missing entry a clean message, not a crash.

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

#[test]
fn no_main_runs_zero_param_fns_and_never_panics() {
    let loft = loft_bin();
    if !fa::exists(&loft) {
        eprintln!("SKIP no_main_runs_zero_param_fns: target/release/loft not built");
        return;
    }
    let dir = std::env::temp_dir().join("loft_no_main_runner");
    let _ = fa::create_dir_all(&dir);
    let src = dir.join("t.loft");
    // A `#native` zero-param void fn (host-import shape, no body) that must be
    // SKIPPED (no crash), alongside two plain zero-param fns that must BOTH run
    // (the #358 contract — the fallback is not `test_`-gated).
    fa::write(
        &src,
        "fn host_thing();\n\
         #native \"loft_test_host_thing\"\n\
         \n\
         fn helper() { print(\"HELPER_RAN\"); }\n\
         fn test_ok() { print(\"TEST_RAN\"); }\n",
    )
    .expect("write source");

    let out = Command::new(&loft)
        .args(["--interpret"])
        .arg(&src)
        .output()
        .expect("invoke loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // The real regression: a used library's `#native` host import must be
    // excluded, not collected-and-executed into a `def(u32::MAX)` panic.
    assert!(
        !stderr.contains("Unknown definition") && !stderr.contains("panicked"),
        "auto runner PANICKED (regression: collected the `#native` host import).\nstderr: {stderr}"
    );
    // Both plain zero-param fns run — the fallback is name-agnostic (#358).
    assert!(
        stdout.contains("TEST_RAN") || stderr.contains("TEST_RAN"),
        "the `test_ok` function did not run.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("HELPER_RAN") || stderr.contains("HELPER_RAN"),
        "a plain zero-param `helper()` did not run (#358 contract broken).\nstdout: {stdout}"
    );
}
