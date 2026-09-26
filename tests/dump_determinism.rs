// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The same source dumps to the same IR and bytecode on every run (loft#1685).
//!
//! A compile is a function of its input, and every before/after comparison in the repo
//! reads it that way: @PLN166's falsifier is `--interpret --dump` over `tests/scripts` +
//! `tests/docs`, byte-identical against the pre-change binary.  Three of those files were
//! not: the scope-exit releases of rebound parameters were handed out by a `std` hash map
//! whose iteration order is seeded per process, so `OpFreeRefIfDistinct(a, __ref_1)`,
//! `(b, __ref_2)` and `(c, __ref_3)` came out in a different order run to run — a valid
//! order every time, and a different dump every time.
//!
//! Each cell dumps one of those three files several times in fresh processes with the
//! program cache off and demands one output.  `LOFT_DUMP_DETERMINISM_BIN` points the cell
//! at another binary, which is how it was falsified: against the binary before the fix
//! (`rebind_orig` still a `HashMap`) `177-reclaim-early-return` read 3 distinct dumps in
//! 6 runs; against the fix, 1.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    std::env::var_os("LOFT_DUMP_DETERMINISM_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_loft")), PathBuf::from)
}

fn dump(file: &str) -> String {
    let out = Command::new(loft_bin())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "60")
        .args(["--interpret", "--dump", file])
        .output()
        .expect("spawn loft");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn same_dump_every_run(file: &str) {
    const RUNS: usize = 6;
    let dumps: BTreeSet<String> = (0..RUNS).map(|_| dump(file)).collect();
    assert!(
        dumps
            .iter()
            .next()
            .is_some_and(|d| d.contains("OpFreeRefIfDistinct")),
        "{file}: the dump no longer carries the releases this guard watches — pick another file"
    );
    assert_eq!(
        dumps.len(),
        1,
        "{file}: {} distinct dumps in {RUNS} runs — a compile must be a function of its input",
        dumps.len()
    );
}

#[test]
fn rebound_parameter_releases_dump_in_one_order() {
    same_dump_every_run("tests/scripts/177-reclaim-early-return.loft");
}

#[test]
fn hoisted_null_buffer_releases_dump_in_one_order() {
    same_dump_every_run("tests/scripts/157-null-buffer-hoist.loft");
}

#[test]
fn caller_buffer_releases_dump_in_one_order() {
    same_dump_every_run("tests/scripts/a-non-escaping-vector-local-is-a-caller-buffer.loft");
}
