// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-SwapIn` — the rebind copy of a minted result into a reset root is an exchange of the
//! two stores, PINNED on the cells (`tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-
//! stores.loft`) by counting the exchanges `LOFT_TRACE_STORE_SWAP=1` names: the cells' values
//! pass on either form, so only this count sees the rule stop firing — on either backend, and
//! not at all under the switch.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-rebind-from-a-fresh-result-exchanges-the-stores.loft";

/// The exchanges one run of the cells makes, and its output.
fn swaps(mode: &str, env: &[(&str, &str)]) -> (usize, String) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg(mode)
        .arg(&src)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TRACE_STORE_SWAP", "1")
        .env_remove("LOFT_NO_STORE_SWAP");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        stderr
            .lines()
            .filter(|l| l.starts_with("[swap-in]"))
            .count(),
        stdout,
    )
}

/// s12's 100 000 rebinds are the floor: each is one exchange on both backends.
#[test]
fn the_rebinds_exchange_their_stores_on_both_backends() {
    for mode in ["--interpret", "--native"] {
        let (n, out) = swaps(mode, &[]);
        assert!(
            out.contains("done"),
            "{mode}: the cells did not finish:\n{out}"
        );
        assert!(
            n >= 100_000,
            "{mode}: {n} exchanges — the rule stopped firing"
        );
    }
}

#[test]
fn the_switch_keeps_every_copy() {
    for mode in ["--interpret", "--native"] {
        let (n, out) = swaps(mode, &[("LOFT_NO_STORE_SWAP", "1")]);
        assert!(
            out.contains("done"),
            "{mode}: the cells did not finish:\n{out}"
        );
        assert_eq!(n, 0, "{mode}: LOFT_NO_STORE_SWAP=1 must keep every copy");
    }
}
