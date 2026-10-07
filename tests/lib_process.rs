// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN179 strand 4 — `lib/process`, on both backends.  The library's own test files carry
// the cells and their hand-computed values: `command.loft` the composition matrix (a value
// can never become syntax), `run.loft` the drain gate (a stream is never left without a
// reader) and the contract of a finished run.  Each must print `ok` on the interpreter AND
// the compiled backend: the native is one body behind two calling conventions, and this is
// what keeps the second one honest.  The composition matrix spawns nothing and runs on every
// host; the drain gate spawns `sh`, `cat` and `printf`.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(backend: &str, file: &str) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--lib")
        .arg(root().join("lib"))
        .arg(root().join("lib/process/tests").join(file))
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{file} on {backend}:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_composition_matrix_holds_interpreted() {
    run("--interpret", "command.loft");
}

#[test]
fn the_composition_matrix_holds_compiled() {
    run("--native", "command.loft");
}

// @PLN184 C2 approved exemption (owner, 2026-10-07): `run.loft` drives `sh -c`, `cat`, `printf` and `/dev/zero`, which have no Windows equivalent; Windows substitute: the composition matrix above
#[cfg(unix)]
mod drain {
    use super::run;

    #[test]
    fn no_stream_is_left_without_a_reader_interpreted() {
        run("--interpret", "run.loft");
    }

    #[test]
    fn no_stream_is_left_without_a_reader_compiled() {
        run("--native", "run.loft");
    }
}
