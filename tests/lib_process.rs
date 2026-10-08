// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN179 strand 4 — `lib/process`, on both backends.  The library's own test files carry
// the cells and their hand-computed values: `command.loft` the composition matrix (a value
// can never become syntax), `run.loft` the drain gate (a stream is never left without a
// reader) and the contract of a finished run, `start.loft` a program running beside this one
// and path holes (@PLN184 P7), `../tests-unix/tree.loft` the tree a stop or a timeout takes
// with it — both also under the emulated Windows host, where a path hole must still reach
// its file.  Each
// must print `ok` on the interpreter AND the compiled backend: the native is one body behind
// two calling conventions, and this is what keeps the second one honest.  The composition
// matrix spawns nothing and runs on every host; the drain gate and `start.loft` spawn `sh`.

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(backend: &str, file: &str) {
    run_on(backend, file, "");
}

fn run_on(backend: &str, file: &str, host: &str) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .env("LOFT_POISON_HOST", host)
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
        "{file} on {backend} (host {host:?}):\n{stdout}\n{}",
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

// @PLN184 C2 approved exemption (owner, 2026-10-07): `run.loft` and `start.loft` drive `sh -c`, `cat`, `printf`, `sleep` and `/dev/zero`, which have no Windows equivalent; Windows substitutes: the composition matrix above, and `windows_rules::a_loft_programs_stop_takes_what_its_child_started`
#[cfg(unix)]
mod spawns_sh {
    use super::{run, run_on};

    #[test]
    fn no_stream_is_left_without_a_reader_interpreted() {
        run("--interpret", "run.loft");
    }

    #[test]
    fn no_stream_is_left_without_a_reader_compiled() {
        run("--native", "run.loft");
    }

    #[test]
    fn a_started_program_and_its_tree_interpreted() {
        run("--interpret", "start.loft");
    }

    #[test]
    fn a_started_program_and_its_tree_compiled() {
        run("--native", "start.loft");
    }

    #[test]
    fn a_stop_takes_the_tree_interpreted() {
        run("--interpret", "../tests-unix/tree.loft");
    }

    #[test]
    fn a_stop_takes_the_tree_compiled() {
        run("--native", "../tests-unix/tree.loft");
    }

    #[test]
    fn a_started_program_and_its_tree_under_the_emulated_windows_host() {
        run_on("--interpret", "start.loft", "windows");
        run_on("--native", "start.loft", "windows");
        run_on("--interpret", "../tests-unix/tree.loft", "windows");
    }
}

// `lines.loft` — the streaming view (@PLN179 strand 4b): the design's gate probes, each a
// cell that can fail on its own — the first line of a slow producer arrives before it ends,
// 50 MB streams through a loop that counts it, a loop left early then `stop()` leaves no
// program behind (its pid asked), and the contract of the finished run.  The producers are
// `sh`, so unix only, like the drain gate above.
#[cfg(unix)]
mod streams_sh {
    use super::run;

    #[test]
    fn lines_stream_as_they_arrive_interpreted() {
        run("--interpret", "lines.loft");
    }

    #[test]
    fn lines_stream_as_they_arrive_compiled() {
        run("--native", "lines.loft");
    }
}
