// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#992 — a droppable local must be released ONCE, after the arm that runs.
//!
//! A `match` in a function's tail position with a `return` in any arm typed the block
//! `never`, and `Scopes::insert_free` sent everything that is not `Void` down the
//! value-returning leg. That leg hoists the tail into a `__ret_N` temp precisely so the
//! tail evaluates BEFORE the frees — but only for a result type it can hoist, and
//! `never` yields no value, so nothing was hoisted and the frees went in front of the
//! tail.
//!
//! [`tests/scripts/992-match-tail-with-return-arm.loft`] carries the shape matrix, using
//! a VALUE read in the arm as its oracle. That oracle is blunt on the interpreter, where
//! a freed store keeps its bytes until something else claims them — only the text cell
//! fails there, against seven on native.
//!
//! This file uses the other oracle, and it is sharp on both: `OpDrop` prints, so the
//! RELEASE lines COUNT the frees and place them relative to the arm body. Against the
//! defect the interpreter prints `RELEASE` before the arm and again at the `return` —
//! a use-after-free — and native panics on the 65535 freed-record marker.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// A `match` in tail position whose taken arm returns. The local is a droppable, so
/// every release announces itself.
const TAKEN_ARM: &str = "struct L992 { id: integer }\n\
fn OpDrop(self: L992) { println(\"RELEASE {self.id}\"); }\n\
fn take992(id: integer) -> L992 { return L992 { id: id }; }\n\
fn d992() { x = take992(32); match 0 { 0 => { println(\"arm {x.id}\"); return }, _ => { } } }\n\
fn main() { d992(); println(\"done\"); }\n";

/// The same with the return in the arm that is NOT taken — the arm that runs must still
/// see a live value, and the release must follow it.
const UNTAKEN_ARM: &str = "struct L992 { id: integer }\n\
fn OpDrop(self: L992) { println(\"RELEASE {self.id}\"); }\n\
fn take992(id: integer) -> L992 { return L992 { id: id }; }\n\
fn d992() { x = take992(32); match 0 { 0 => { println(\"arm {x.id}\") }, _ => { return } } }\n\
fn main() { d992(); println(\"done\"); }\n";

/// Every arm returns, so the block genuinely never completes. Each arm frees for itself;
/// nothing may free ahead of them.
const EVERY_ARM: &str = "struct L992 { id: integer }\n\
fn OpDrop(self: L992) { println(\"RELEASE {self.id}\"); }\n\
fn take992(id: integer) -> L992 { return L992 { id: id }; }\n\
fn d992() { x = take992(32); match 0 { 0 => { println(\"arm {x.id}\"); return }, _ => { return } } }\n\
fn main() { d992(); println(\"done\"); }\n";

fn write_probe(tag: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("loft_992_{tag}_{}.loft", std::process::id()));
    fa::write(&path, src).expect("write probe");
    path
}

fn run(backend: &str, file: &PathBuf) -> (bool, String, String) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// One release, and it comes after the arm body.
fn assert_released_once_after_the_arm(tag: &str, src: &str, backend: &str) {
    let probe = write_probe(&format!("{tag}_{}", backend.trim_start_matches('-')), src);
    let (ok, stdout, stderr) = run(backend, &probe);
    let _ = fa::remove_file(&probe);
    assert!(
        ok,
        "[{tag}/{backend}] the program must run to completion — a second release of the \
         same record is a use-after-free, which native reports as an out-of-bounds index \
         on the 65535 freed-record marker\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let releases = stdout.matches("RELEASE 32").count();
    assert_eq!(
        releases, 1,
        "[{tag}/{backend}] the local owns one record and must be released exactly once\n\
         stdout:\n{stdout}"
    );
    let arm = stdout.find("arm 32").unwrap_or_else(|| {
        panic!("[{tag}/{backend}] the arm must read a LIVE value\nstdout:\n{stdout}")
    });
    let release = stdout.find("RELEASE 32").expect("release line present");
    assert!(
        release > arm,
        "[{tag}/{backend}] the release must follow the arm that reads the value, not \
         precede it\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("done"),
        "[{tag}/{backend}] the caller must resume after the call\nstdout:\n{stdout}"
    );
}

#[test]
fn taken_return_arm_releases_once_interpret() {
    assert_released_once_after_the_arm("taken", TAKEN_ARM, "--interpret");
}

#[test]
fn taken_return_arm_releases_once_native() {
    assert_released_once_after_the_arm("taken", TAKEN_ARM, "--native");
}

#[test]
fn untaken_return_arm_releases_once_interpret() {
    assert_released_once_after_the_arm("untaken", UNTAKEN_ARM, "--interpret");
}

#[test]
fn untaken_return_arm_releases_once_native() {
    assert_released_once_after_the_arm("untaken", UNTAKEN_ARM, "--native");
}

#[test]
fn return_in_every_arm_releases_once_interpret() {
    assert_released_once_after_the_arm("every", EVERY_ARM, "--interpret");
}

#[test]
fn return_in_every_arm_releases_once_native() {
    assert_released_once_after_the_arm("every", EVERY_ARM, "--native");
}
