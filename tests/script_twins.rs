// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN179 strand 2 — the twin cases the repository declares, run on every PR: each port's
// `tests/comparisons/scripts/<port>/cases.tsv` is one or more invocations of
// `scripts/script_twin.sh` comparing the ORIGINAL script and its port on four channels, on
// both backends.  Before this nothing re-ran a twin after it first went green, and a port
// (`scripts/wasm_bundle_stamp`) broke silently under a later language change.  The cases
// marked `slow` are left out here (`make script-twins ARGS=--slow` runs them).  Unix: the
// originals are bash and Python, the harness is bash.

#[cfg(unix)]
#[test]
fn every_declared_twin_case_agrees_on_both_backends() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
