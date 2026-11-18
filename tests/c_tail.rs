// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-CTail`: `tests/c_tail/exits.loft` — functions leaving through several exits, one
//! per result shape — answers its hand-computed values on the interpreter, on the C backend
//! with each shared release tail written once, and with the pass off; it exits 0 and frees
//! every store each way.  The trace shows the functions the cell is about really were
//! shortened, so the cell does not pass by staying as written.
//!
//! @falsified-at: d66006112 — scored by hand against two defects planted in `src/c/tail.rs`.
//!   CHANNEL: value (the cell's own assertions) and leak (`stores not freed`), on `--native-c`.
//!   ARMED: nothing to arm: the pass runs by default.
//!   WITNESS: (a) a shared end's label put one statement late (`at.push(first + k + 1)`), so a
//!   jump skips the first shared release: the cell answers and then reports `2 stores not
//!   freed at program exit: kt=100 P×2`.  (b) an exit's own statements not written
//!   (`&exit.stmts[..0]`), so it jumps without its own releases or its read of the result: the
//!   cell prints nothing and exits 1.
//!   HOLDS: the interpreter run and the `LOFT_NO_C_TAIL=1` run pass on both planted trees.

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn cell() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/c_tail/exits.loft")
}

fn have_cc() -> bool {
    loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn run(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.args(args)
        .arg("--no-warnings")
        .arg(cell())
        .env("LOFT_NATIVE_LEAK_CHECK", "1")
        .env_remove("LOFT_NO_C_TAIL")
        .env_remove("LOFT_TRACE_C_TAIL");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("loft runs");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        o.status.success(),
    )
}

#[test]
fn every_exit_answers_and_frees_with_its_tail_shared() {
    let (out, err, ok) = run(&["--interpret"], &[]);
    assert!(ok && out == "exits ok\n", "on the interpreter: {out}{err}");
    if !have_cc() {
        return;
    }
    let (out, err, ok) = run(&["--native-c"], &[("LOFT_TRACE_C_TAIL", "1")]);
    assert!(ok && out == "exits ok\n", "tails shared: {out}{err}");
    assert!(
        !err.contains("stores not freed"),
        "tails shared, a leak: {err}"
    );
    for f in ["n_txt", "n_eff"] {
        let line = format!("[c-tail]\t{f}\t");
        assert!(
            err.lines().any(|l| l.starts_with(&line)),
            "`{f}` kept every tail: {err}"
        );
    }
    let (out, err, ok) = run(&["--native-c"], &[("LOFT_NO_C_TAIL", "1")]);
    assert!(ok && out == "exits ok\n", "the pass off: {out}{err}");
    assert!(
        !err.contains("[c-tail]"),
        "the switch left the pass on: {err}"
    );
}
