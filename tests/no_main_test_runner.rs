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

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
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

    let out = loft::platform::process::harness_command(&loft)
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

/// Run `src` (written to a file of its own) on the interpreter: its exit code and stderr.
fn run_no_main(name: &str, src: &str) -> Option<(i32, String)> {
    let loft = loft_bin();
    if !fa::exists(&loft) {
        eprintln!("SKIP {name}: target/release/loft not built");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("loft_no_main_{name}"));
    let _ = fa::create_dir_all(&dir);
    let file = dir.join("t.loft");
    fa::write(&file, src).expect("write source");
    let out = loft::platform::process::harness_command(&loft)
        .args(["--interpret"])
        .arg(&file)
        .output()
        .expect("invoke loft");
    Some((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// A failed `assert` in one of the functions the fallback runs is a failed run: reported
/// with the function's name, the next function still run, and a non-zero exit — as the C
/// runner answers.  Each function runs in a state of its own, and the check after the run
/// read only the outer one, so the interpreter exited 0 with nothing printed.
#[test]
fn a_failed_assert_without_main_fails_the_run() {
    let Some((code, stderr)) = run_no_main(
        "failed_assert",
        "fn test_bad() { assert(1 + 1 == 3, \"three\"); }\n\
         fn test_good() { assert(1 + 1 == 2, \"two\"); }\n",
    ) else {
        return;
    };
    assert_eq!(code, 1, "a failed assert exited {code}.\nstderr: {stderr}");
    assert!(
        stderr.contains("assertion failed: three") && stderr.contains("FAIL test_bad"),
        "the failure was not reported.\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("FAIL test_good"),
        "a passing test was reported.\nstderr: {stderr}"
    );
    // `@EXPECT_FAIL` turns it round: the run passes only by failing with that message.
    let Some((code, stderr)) = run_no_main(
        "expected_fail",
        "fn test_ok() { }\n\
         // @EXPECT_FAIL: three\n\
         fn test_bad() { assert(1 + 1 == 3, \"three\"); }\n",
    ) else {
        return;
    };
    assert_eq!(
        code, 0,
        "an expected failure failed the run.\nstderr: {stderr}"
    );
}

/// A file directive with an argument (`#width 32`, `#link static`) heading a file of
/// definitions with no `fn main` is a directive, not a loose statement: the file is no
/// beginner script, so it parses as written and its functions run.  It was refused with
/// "Expect token ;" — every `#width 32` test file of the language suite with no `main`.
#[test]
fn a_file_directive_heads_a_file_without_main() {
    for (name, head) in [("width32", "#width 32"), ("link", "#link static")] {
        let Some((code, stderr)) = run_no_main(
            name,
            &format!("{head}\nfn test_a() {{ assert(2147483647 + 0 == 2147483647, \"edge\"); }}\n"),
        ) else {
            return;
        };
        assert_eq!(
            code, 0,
            "`{head}` without main exited {code}.\nstderr: {stderr}"
        );
    }
    // Under the regime, the function does run its 32-bit arithmetic.
    let Some((code, stderr)) = run_no_main(
        "width32_runs",
        "#width 32\nfn test_a() { a = 2147483647; assert(a + 1 == null, \"wrapped\"); }\n",
    ) else {
        return;
    };
    assert_eq!(
        code, 0,
        "the regime's arithmetic did not hold.\nstderr: {stderr}"
    );
}
