// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DispatchStop` — the interpreter's lean dispatch loop tests ONE flag after each op
//! instead of every rare event that ends it, and each event sets the flag where it happens.
//! What that can break is an event that no longer reaches the loop: a runtime error that does
//! not halt, so the program runs on past it.  These cells put each raise path inside a loop
//! in a callee and assert nothing runs after the fault: a NATIVE raise (`assert`, through
//! `State::invoke_native`) and an OP raise (the call-depth cap, through `State::raise_at`).
//! The `par` worker's fault is `runtime_errors.rs`'s loft#1053 cases, the frame yield
//! `dispatch_reentry.rs`.  And the one structural fact the flag rests on — a runtime error is
//! stored in exactly one place, `Stores::raise_runtime_error`, which sets the flag with it —
//! is checked over the source, so a new raise site cannot bypass it.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(name: &str, src: &str) -> (String, String, bool) {
    let dir = std::env::temp_dir().join(format!("loft_dispatch_stop_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join(format!("{name}.loft"));
    fa::write(&file, src).expect("write program");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let _ = fa::remove_file(&file);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn a_native_raise_inside_a_loop_halts_at_once() {
    let (out, err, ok) = run(
        "native_raise",
        "fn inner(n: integer) -> integer {\n  t = 0;\n  for i in 0..n {\n    t += i;\n    \
         assert(i < 5, \"stopped at {i}\");\n  }\n  t\n}\n\
         fn main() {\n  for r in 0..3 {\n    println(\"round {r}\");\n    x = inner(10);\n    \
         println(\"after {x}\");\n  }\n}\n",
    );
    assert!(!ok, "the run must fail\nstdout: {out}\nstderr: {err}");
    assert!(
        err.contains("stopped at 5"),
        "the assertion is reported: {err}"
    );
    assert_eq!(out.trim(), "round 0", "nothing runs after the fault: {out}");
}

#[test]
fn an_op_raise_inside_a_loop_halts_at_once() {
    let (out, err, ok) = run(
        "op_raise",
        "fn deep(n: integer) -> integer {\n  deep(n + 1) + 1\n}\n\
         fn main() {\n  for r in 0..3 {\n    println(\"round {r}\");\n    x = deep(0);\n    \
         println(\"after {x}\");\n  }\n}\n",
    );
    assert!(!ok, "the run must fail\nstdout: {out}\nstderr: {err}");
    assert!(
        err.to_lowercase().contains("stack"),
        "the call-depth fault is reported: {err}"
    );
    assert_eq!(out.trim(), "round 0", "nothing runs after the fault: {out}");
}

/// A runtime error is stored in one place, and that place sets the flag.
#[test]
fn a_runtime_error_is_stored_only_by_its_setter() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fa::read_dir(&dir).expect("read src") {
            let path = entry.os_spelling();
            if fa::is_dir(&path) {
                stack.push(path);
            } else if fa::has_extension(&path, "rs") {
                let text = fa::read_to_string(&path).expect("read file");
                for (n, line) in text.lines().enumerate() {
                    if line.contains("runtime_error = Some(")
                        && !line.trim_start().starts_with("//")
                    {
                        sites.push(format!("{}:{}", loft::file_access::portable(&path), n + 1));
                    }
                }
            }
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "`runtime_error` is assigned outside `Stores::raise_runtime_error`, so the dispatch \
         loop's flag (@FR-R-DispatchStop) would not see it: {sites:?}"
    );
    assert!(
        sites[0].contains("database/mod.rs"),
        "the one assignment is the setter's: {sites:?}"
    );
}
