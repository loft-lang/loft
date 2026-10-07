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

use loft::file_access as fa;
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

// approved exemption: `run.loft` and `start.loft` drive `sh -c`, `cat`, `printf`, `sleep` and `/dev/zero`, which have no Windows equivalent; Windows substitutes: the composition matrix above, and `windows_rules::a_loft_programs_stop_takes_what_its_child_started`
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

    // `lines.loft` — the streaming view (@PLN179 strand 4b): the design's gate probes, each
    // a cell that can fail on its own — the first line of a slow producer arrives before it
    // ends, 50 MB streams through a loop that counts it, a loop left early then `stop()`
    // leaves no program behind (its pid asked), and the contract of the finished run.  The
    // producers are `sh`, under the same exemption as the drain gate.
    #[test]
    fn lines_stream_as_they_arrive_interpreted() {
        run("--interpret", "lines.loft");
    }

    #[test]
    fn lines_stream_as_they_arrive_compiled() {
        run("--native", "lines.loft");
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

    // @PLN179 strand 2 — the twin cases the repository declares, run on every PR: each
    // port's `tests/comparisons/scripts/<port>/cases.tsv` is one or more invocations of
    // `scripts/script_twin.sh` comparing the ORIGINAL script and its port on four channels,
    // on both backends.  Before this nothing re-ran a twin after it first went green, and a
    // port (`scripts/wasm_bundle_stamp`) broke silently under a later language change.  The
    // cases marked `slow` are left out here (`make script-twins ARGS=--slow` runs them).
    // Here, under this module's exemption: the originals are bash and Python, the harness
    // is bash — the plan's Windows verdict for a port is still to build.
    #[test]
    fn every_declared_twin_case_agrees_on_both_backends() {
        let root = super::root();
        let have = |tool: &str| {
            loft::platform::process::harness_command(tool)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !have("python3") {
            println!("script_twins: skipped (no python3 for the Python originals)");
            return;
        }
        let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_loft"))
            .parent()
            .expect("the loft binary's directory");
        let path = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = loft::platform::process::harness_command("bash");
        cmd.arg(root.join("scripts/script_twins.sh"))
            .current_dir(&root)
            .env("PATH", path)
            .env("LOFT_TIMEOUT", "300");
        if !have("rustc") {
            cmd.arg("--interpret");
        }
        let out = cmd.output().expect("run scripts/script_twins.sh");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "a twin case went red — the original and its port no longer leave the same world:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("twin run(s), 0 red") && !stdout.contains(" 0 twin run(s)"),
            "the runner ran nothing or did not report:\n{stdout}"
        );
    }
}

/// A script names `Command` and nothing else: the TYPE trigger loads `process` (@PLN179
/// strand 4, PROCESS.md § Where it lives).  The package is a path dependency of the
/// script's own manifest, as `scripts/loft.toml` declares it; the literal goes through the
/// typed-format hook (a hole with spaces stays one word), which is the second half of the
/// same fix — a type imported by name kept its `lit` / `hole_*` out of the hook's reach.
/// Both backends, from a working directory that is not the package's.
#[test]
fn a_type_trigger_loads_the_library_with_nothing_naming_it() {
    let dir = std::env::temp_dir().join(format!("loft_type_trigger_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    fa::write(
        dir.join("loft.toml"),
        format!(
            "[package]\nname = \"trig\"\nversion = \"0.1.0\"\n\n[dependencies]\nprocess = {{ path = \"{}\" }}\n",
            root().join("lib/process").display()
        ),
    )
    .expect("manifest");
    fa::write(
        dir.join("p.loft"),
        "fn main() {\n  w = \"a b\";\n  c: Command = \"printf %s|{w}|\";\n  r = c.run();\n  println(\"{r.ok} {r.stdout}\");\n}\n",
    )
    .expect("script");
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(dir.join("p.loft"))
            .env("LOFT_TIMEOUT", "120")
            .current_dir(std::env::temp_dir())
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.trim_end() == "true |a b|",
            "[{backend}] the type trigger did not load `process`, or the hook did not fire:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
