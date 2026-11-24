// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: reads the Rust writer's overflow memo — its `--native` runs as `--native-rust`.
//! @PLN157 § V-ao (`@FR-R-Invariant`) — the EMISSION pins.  An integer chain a loop cannot
//! change (`+ - * neg & | ^` over literals and variables the loop neither rebinds nor lets
//! escape) is evaluated at its first use and answered from a memo after; the memo is
//! declared at the innermost loop that spells the chain; a rebound leaf, an escaped leaf
//! (a by-reference or fn-ref argument), a field read and a chain of literals alone decline;
//! one chain spelled twice is one memo; `LOFT_NO_INVARIANT_HOIST=1` restores the per-use
//! evaluation everywhere and `LOFT_HOIST_VERIFY=1` asserts the memo at every use.  The
//! overflow note fires where the FIRST evaluation stands — `LOFT_DEV_SOFT_HALT=1` counts
//! it.  The cell corpus (`bytecode-comparisons/V-ao-invariant-arith-cells.loft`) says the
//! VALUES hold on both backends; this pins what is emitted.
//!
//! Beside it, loft#1534: the non-sentinel proof's escape collector reads a by-reference
//! argument through its `OpCreateStack` spelling, so a callee's overflow into a local the
//! proof trusted answers `null` on native as it does on the interpreter.
use loft::file_access as fa;
use std::path::PathBuf;

/// Run `src` as a native program with `env`, answering (stdout, stderr).
fn run_native(tag: &str, src: &str, env: &[(&str, &str)]) -> (String, String) {
    let file = std::env::temp_dir().join(format!(
        "loft_invariant_arith_{}_{tag}.loft",
        std::process::id()
    ));
    fa::write(&file, src).expect("write the probe");
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg(&file)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_INVARIANT_HOIST")
        .env_remove("LOFT_NO_NN_FAST");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft --native");
    let _ = fa::remove_file(&file);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// loft#1534 — a local handed to a `&integer` parameter escapes the non-sentinel proof
/// through its `OpCreateStack` spelling: the callee's overflow leaves it null, and `k + 1`
/// answers null on native as the interpreter does (the proven form answered a number).
#[test]
fn a_by_reference_argument_escapes_the_non_sentinel_proof() {
    let src = "fn poison(n: &integer) { n = n * 4; }\n\
               fn main() { k = 1; k = 4611686018427387904; poison(k); r = k + 1; println(\"r {r}\"); }\n";
    let (out, err) = run_native("escape", src, &[]);
    assert!(
        out.contains("r null"),
        "loft#1534: native must answer null:\n{out}\n{err}"
    );
}
