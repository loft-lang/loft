// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `[x; n]` and the constant comprehension — the EMISSION pins.  A constant comprehension in a
//! struct FIELD lowers to the repeat literal (one template append and one `OpAppendCopy`
//! addressed to the FIELD, not the record), as a local's has since loft#884;
//! `LOFT_NO_FIELD_FILL=1` keeps the per-element loop.  The runtime fill behind the op is one
//! doubling block copy (`Stores::fill_from_template`); `LOFT_NO_BLOCK_REPEAT=1` restores the
//! per-element copy.  The cell corpus (`tests/scripts/a-repeated-element-fills-in-one-block.loft`)
//! says the VALUES hold on both backends under both switches; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-repeated-element-fills-in-one-block.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_FIELD_FILL")
        .env_remove("LOFT_NO_BLOCK_REPEAT");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let s = src.to_str().unwrap().to_string();
    let mut a: Vec<&str> = args.to_vec();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_values_hold_on_both_backends_under_both_switches_and_the_falsifiers() {
    run_ok(&["--interpret"], &[]);
    run_ok(
        &["--native"],
        &[
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ],
    );
    run_ok(&["--native"], &[("LOFT_NO_BLOCK_REPEAT", "1")]);
    run_ok(&["--interpret"], &[("LOFT_NO_BLOCK_REPEAT", "1")]);
    run_ok(&["--native"], &[("LOFT_NO_FIELD_FILL", "1")]);
}
