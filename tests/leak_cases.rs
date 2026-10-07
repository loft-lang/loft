// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Cross-backend store-leak case suite.
//!
//! Every `.loft` file under `tests/leak_cases/<folder>/` is run through
//! BOTH backends — the interpreter (`--interpret`) and native
//! (`--native`, with `LOFT_NATIVE_LEAK_CHECK=1`) — and checked against
//! the leak behaviour its folder declares.  See
//! `tests/leak_cases/README.md` for the folder → expectation mapping.
//!
//! This is the permanent both-backend layer that `tests/leak.rs` (an
//! interpreter-only, in-process suite) lacked: before @P297, native had
//! no exit-time leak check at all, so native-only leaks (e.g. @P298)
//! went undetected.  Drop a `.loft` file in `clean/` and it is guarded
//! on native too.
//!
//! - `leak_cases_interp` runs in the default `cargo test` path (cheap).
//! - `leak_cases_native` is `#[ignore]`d (heavy — `rustc` per case):
//!
//! ```bash
//! cargo test --release --test leak_cases -- --ignored
//! ```

use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Expected leak behaviour for a case folder.  `None` = not a case
/// folder (e.g. the README lives at the top level, ignored).
#[derive(Clone, Copy)]
struct Expect {
    interp_leaks: bool,
    native_leaks: bool,
}

fn folder_expect(folder: &str) -> Option<Expect> {
    match folder {
        "clean" => Some(Expect {
            interp_leaks: false,
            native_leaks: false,
        }),
        // Open-bug folders, suffix-driven (see tests/leak_cases/README.md):
        //   known_<id>_both   — leaks on BOTH backends (e.g. the pre-fix @P297)
        //   known_<id>_native — native-only leak    (e.g. the pre-fix @P298)
        // A case asserts its leak still reproduces; when fixed, move it to clean/.
        // There are currently none — @P297/@P298 were fixed 2026-05-21.
        f if f.starts_with("known_") && f.ends_with("_both") => Some(Expect {
            interp_leaks: true,
            native_leaks: true,
        }),
        f if f.starts_with("known_") && f.ends_with("_native") => Some(Expect {
            interp_leaks: false,
            native_leaks: true,
        }),
        _ => None,
    }
}

/// `(folder, file_stem, path)` for every `.loft` case.
fn case_files() -> Vec<(String, String, PathBuf)> {
    let base = root().join("tests/leak_cases");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&base).expect("tests/leak_cases must exist") {
        let dir = entry.unwrap().path();
        if !fa::is_dir(&dir) {
            continue;
        }
        let folder = dir.file_name().unwrap().to_string_lossy().into_owned();
        if folder_expect(&folder).is_none() {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap() {
            let p = f.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) == Some("loft") {
                let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
                out.push((folder.clone(), stem, p));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "no leak cases found under tests/leak_cases"
    );
    out.sort();
    out
}

/// Wrap a case body (`fn test()`) into a runnable program in a temp file.
fn wrap_to_temp(name: &str, path: &Path) -> PathBuf {
    let body = fa::read_to_string(path).unwrap();
    let src = format!("{body}\nfn main() {{ test(); }}\n");
    let tmp = std::env::temp_dir().join(format!("loft_leakcase_{name}.loft"));
    fa::write(&tmp, src).unwrap();
    tmp
}

/// Returns `(success, leaked, stderr)`.
fn run(mode: &str, src: &Path, leak_env: bool) -> (bool, bool, String) {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(mode).arg(src).current_dir(root());
    if leak_env {
        cmd.env("LOFT_NATIVE_LEAK_CHECK", "1");
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("spawn loft {mode}: {e}"));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (
        out.status.success(),
        stderr.contains("stores not freed"),
        stderr,
    )
}

/// Shared driver: run every case in `mode`, comparing the observed leak
/// against `expected(folder)`, and report ALL mismatches at once.
fn sweep(mode: &str, leak_env: bool, expected: impl Fn(Expect) -> bool) {
    let mut failures = Vec::new();
    for (folder, name, path) in case_files() {
        let exp = folder_expect(&folder).unwrap();
        let want_leak = expected(exp);
        let tmp = wrap_to_temp(&name, &path);
        let (ok, leaked, stderr) = run(mode, &tmp, leak_env);
        let _ = fa::remove_file(&tmp);
        if !ok {
            failures.push(format!(
                "{folder}/{name}: {mode} run failed\n---- stderr ----\n{stderr}"
            ));
            continue;
        }
        if leaked != want_leak {
            failures.push(format!(
                "{folder}/{name}: {mode} expected leaks={want_leak}, observed leaks={leaked}\
                 \n  (if a fix flipped this, move the file to the matching folder)"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} leak-case mismatch(es) in {mode}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// @speed 2.3
#[test]
fn leak_cases_interp() {
    sweep("--interpret", false, |e| e.interp_leaks);
}

#[test]
#[ignore = "heavy (rustc per case) — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test leak_cases leak_cases_native -- --ignored`"]
fn leak_cases_native() {
    sweep("--native", true, |e| e.native_leaks);
}
