// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A test file compiled from a SHARED library parse must answer exactly what it
//! answered when it parsed that library for itself (loft#925).
//!
//! `loft test` used to load a `use`d library from source once per test file —
//! twice over, since both parse passes re-run the use region — so a suite paid the
//! PRODUCT of its file count and its library's size.  It now parses each distinct
//! `use` region once and starts every later file in that group from a copy.
//!
//! The whole risk of that lives in one word: *exactly*.  A base that quietly stops
//! asserting is worse than the seconds it saves, and the assertions at stake are
//! the ones a library's CI is built on — `@EXPECT_ERROR`, `@EXPECT_WARNING`,
//! `--deny-warnings`, the diagnostics the library itself raises.  So the guard here
//! is byte equality of the WHOLE run against `LOFT_NO_TEST_BASE=1`, the same binary
//! with the sharing switched off, over a package shaped to reach every one of those
//! paths at once.
//!
//! The second thing a base could get wrong is seeing too much: a file that names
//! one library must not resolve a name from another, or a compile that should fail
//! silently succeeds.  Hence four groups over two libraries, interleaved in the
//! directory so the group a file belongs to is never the file next to it.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A package with two libraries and four `use` groups over them.
///
/// Every test file is a group member (each region is written by at least two
/// files), because a base is only built once a second file asks for one — a
/// fixture of singletons would exercise nothing.
///
/// `who` gives each TEST its own root, and that is load-bearing rather than tidy:
/// this function begins by REMOVING the root, the tests in this binary run in
/// parallel by default, and `std::process::id()` is the same value for all of
/// them — so a root keyed on the pid alone is one directory that every test
/// wipes.  One test's removal then lands between another's `create_dir_all` and
/// its first write, or under a run already in progress.  Measured on the
/// pid-only form: six failures in eighteen runs under concurrency (panics at the
/// write below and inside `the_shared_base_is_built_for_every_group`), against
/// eight clean runs when the binary ran alone — a flake that only appears on a
/// loaded box, which is where gates run.
fn fixture(who: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_925_{}_{who}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::create_dir_all(root.join("tests")).expect("mkdir tests");
    let w = |rel: &str, body: &str| {
        std::fs::write(root.join(rel), body).unwrap_or_else(|e| panic!("write {rel}: {e}"));
    };

    w(
        "loft.toml",
        "[package]\nname = \"base925\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/base925.loft\"\n",
    );
    // The aggregator is the entry, so `use base925;` is what a consumer writes.
    w("src/base925.loft", "use alpha;\nuse beta;\n");
    // `alpha` raises a diagnostic of its own — the one a per-file parse no longer
    // sees, and therefore the one a base has to carry forward.
    w(
        "src/alpha.loft",
        "pub struct Point { x: integer, y: integer }\n\
         pub fn alpha_add(a: integer, b: integer) -> integer { a + b }\n\
         pub fn alpha_unread(a: integer, idx: integer) -> integer { a + 1 }\n",
    );
    w(
        "src/beta.loft",
        "pub fn beta_mul(a: integer, b: integer) -> integer { a * b }\n",
    );

    // Group A — `use alpha;`.  Interleaved with the others by file name, since the
    // runner walks a directory in sorted order.
    w(
        "tests/t1_a.loft",
        "use alpha;\nfn test_add() { assert(alpha_add(2, 3) == 5, \"add\"); }\n",
    );
    w(
        "tests/t2_b.loft",
        "use beta;\nfn test_mul() { assert(beta_mul(2, 3) == 6, \"mul\"); }\n",
    );
    // Group C — the aggregator, which pulls both libraries in transitively.
    w(
        "tests/t3_c.loft",
        "use base925;\nfn test_both() { assert(alpha_add(1, beta_mul(2, 2)) == 5, \"both\"); }\n",
    );
    // The library's own warning must still be reported against a file that only
    // `use`s it — this is the assertion a dropped base diagnostic would silence.
    w(
        "tests/t4_a.loft",
        "// @EXPECT_WARNING: Parameter idx is never read\n\
         use alpha;\n\
         fn test_warned() { assert(alpha_unread(1, 9) == 2, \"unread\"); }\n",
    );
    // A file that names ONE library must not see the other's names.
    w(
        "tests/t5_b.loft",
        "// @EXPECT_ERROR: Unknown function alpha_add\n\
         use beta;\n\
         fn test_cross() { x = alpha_add(1, 1); assert(x == 2, \"must not resolve\"); }\n",
    );
    // The annotations a library's CI leans on, over the shared base.
    w(
        "tests/t6_c.loft",
        "// @EXPECT_FAIL: on purpose\n\
         use base925;\n\
         fn test_fails() { assert(alpha_add(1, 1) == 3, \"on purpose\"); }\n",
    );
    w(
        "tests/t7_a.loft",
        "use alpha;\n\
         fn test_runs() { assert(alpha_add(4, 4) == 8, \"runs\"); }\n\
         // @IGNORE\n\
         fn test_skipped() { assert(false, \"never runs\"); }\n",
    );
    // A `use` region spread over a comment and a blank line still names its group.
    w(
        "tests/t8_c.loft",
        "// a comment above the use region\n\n\
         use base925;\n\n\
         fn test_spaced() { assert(beta_mul(3, 3) == 9, \"spaced\"); }\n",
    );
    // The cross-check in the other direction, and LAST in the directory — so a
    // base that leaked forward from any earlier group is caught here.  One
    // direction alone is not enough: a leak has an order, and the file that would
    // notice it has to come after the group that leaks.
    w(
        "tests/t9_a.loft",
        "// @EXPECT_ERROR: Unknown function beta_mul\n\
         use alpha;\n\
         fn test_cross_back() { x = beta_mul(2, 2); assert(x == 4, \"must not resolve\"); }\n",
    );
    // A leading `#cwd` is part of the region, not a reason to refuse one.  This is
    // the shape the reporting consumer actually writes — all 81 of its test files
    // open with it — so a version that gave up here would have measured green on a
    // synthetic and saved that consumer nothing.  Its own group, since the region
    // is the key and this text differs from a bare `use alpha;`.
    w(
        "tests/ta_d.loft",
        "#cwd\n\nuse alpha;\n\nfn test_cwd_one() { assert(alpha_add(5, 5) == 10, \"cwd1\"); }\n",
    );
    w(
        "tests/tb_d.loft",
        "#cwd\n\nuse alpha;\n\nfn test_cwd_two() { assert(alpha_add(6, 6) == 12, \"cwd2\"); }\n",
    );
    root
}

fn run(root: &PathBuf, extra: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.current_dir(root)
        .args(["test", "tests"])
        .env("LOFT_TIMEOUT", "180");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run loft test");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// The guard: the same binary, the same package, the sharing on and off.
#[test]
fn a_shared_library_parse_answers_what_a_private_one_did() {
    let root = fixture("equivalence");
    let (shared_code, shared) = run(&root, &[]);
    let (private_code, private) = run(&root, &[("LOFT_NO_TEST_BASE", "1")]);

    // Not vacuous: the run has to have reached every annotation under test, or
    // "identical" would only be saying that two empty runs are empty.
    for marker in [
        "Parameter idx is never read", // the library's diagnostic, carried forward
        "t4_a.loft",
        "t5_b.loft", // the cross-library name, refused
        "t6_c.loft", // @EXPECT_FAIL
        "t7_a.loft", // @IGNORE
        "t8_c.loft",
        "t9_a.loft", // the reverse cross-library check
        "tb_d.loft", // the `#cwd` region — the reporting consumer's shape
    ] {
        assert!(
            private.contains(marker),
            "the control run never reached `{marker}`, so equality proves nothing:\n{private}"
        );
    }
    assert_eq!(
        private, shared,
        "a test file answered differently when its library came from a shared parse"
    );
    assert_eq!(private_code, shared_code, "exit code differs");
}

/// The equality above is only evidence while the sharing actually happens: a base
/// that silently stopped being built would leave it comparing a run to itself.
#[test]
fn the_shared_base_is_built_for_every_group() {
    let root = fixture("base_built");
    let (_, out) = run(&root, &[("LOFT_TEST_BASE_REPORT", "1")]);
    for region in ["use alpha;", "use beta;", "use base925;", "#cwd use alpha;"] {
        assert!(
            out.contains(&format!("test base shared — {region}")),
            "no shared base for `{region}` — the equivalence guard is comparing a \
             run with sharing off to a run with sharing off:\n{out}"
        );
    }
    // One per group, and no more: a base rebuilt per file would be the bug this
    // whole change exists to remove.
    assert_eq!(
        out.matches("test base shared").count(),
        4,
        "expected exactly one base per `use` group:\n{out}"
    );
}

/// `--deny-warnings` is the gate a library's CI runs, and it reads the diagnostics
/// the parse produced.  A base that dropped the library's warning would turn a red
/// CI green — the failure mode that makes this whole change worth verifying.
#[test]
fn deny_warnings_still_sees_the_librarys_own_warning() {
    let root = fixture("deny_warnings");
    let (shared_code, shared) = run(&root, &[("LOFT_DENY_WARNINGS", "1")]);
    let (private_code, private) = run(
        &root,
        &[("LOFT_DENY_WARNINGS", "1"), ("LOFT_NO_TEST_BASE", "1")],
    );
    assert!(
        private.contains("Parameter idx is never read"),
        "the control never raised the library warning:\n{private}"
    );
    assert_eq!(
        private, shared,
        "--deny-warnings differs under a shared base"
    );
    assert_eq!(private_code, shared_code, "exit code differs");
}
