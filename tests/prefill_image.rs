// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C4 (`@FR-R-Prefill`) — THE PREFILL IMAGE: the default prefill of a minted record
//! is one block write of a per-type image captured from the field-by-field walk's first
//! run, never the walk again.  The cell corpus
//! (`plans/164-activation-arena/bytecode-comparisons/C4-prefill-image-cells.loft`) says the
//! VALUES hold on both backends; this pins the INSTRUMENT and the PROTOCOL — every mint of
//! the cells is checked against the walk under `LOFT_PREFILL_VERIFY=1` on both backends
//! (the image is what the walk writes, or the run panics naming the type), the output is
//! identical with `LOFT_NO_PREFILL_IMAGE=1`, and the image changes no store mint.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/C4-prefill-image-cells.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "200")
        .env_remove("LOFT_NO_PREFILL_IMAGE")
        .env_remove("LOFT_PREFILL_VERIFY")
        .env_remove("LOFT_TRACE_PREFILL");
    cmd
}

/// Run the cells in `mode` with `env`; the stdout, after asserting a clean exit.
fn run(mode: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg(mode).arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{mode} {env:?} failed (exit {:?}):\n{stderr}",
        out.status
    );
    assert!(
        !stderr.contains("disagrees with the walk"),
        "{mode} {env:?}: the image disagreed with the walk:\n{stderr}"
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const VERIFY: &[(&str, &str)] = &[
    ("LOFT_PREFILL_VERIFY", "1"),
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
];
const VERIFY_NATIVE: &[(&str, &str)] = &[
    ("LOFT_PREFILL_VERIFY", "1"),
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
    ("LOFT_NATIVE_LEAK_CHECK", "1"),
];
const OFF: &[(&str, &str)] = &[("LOFT_NO_PREFILL_IMAGE", "1")];

#[test]
fn every_mint_of_the_cells_matches_the_walk_on_the_interpreter() {
    let out = run("--interpret", VERIFY);
    assert!(
        out.trim_end().ends_with("ok"),
        "cells did not reach `ok`:\n{out}"
    );
}

#[test]
fn every_mint_of_the_cells_matches_the_walk_on_native() {
    let out = run("--native", VERIFY_NATIVE);
    assert!(
        out.trim_end().ends_with("ok"),
        "cells did not reach `ok`:\n{out}"
    );
}

#[test]
fn the_switch_answers_the_same_values() {
    let on = run("--interpret", &[]);
    let off = run("--interpret", OFF);
    assert_eq!(
        on, off,
        "interpreter: the image and the walk must print the same"
    );
    let n_on = run("--native", &[]);
    let n_off = run("--native", OFF);
    assert_eq!(
        n_on, n_off,
        "native: the image and the walk must print the same"
    );
    assert_eq!(on, n_on, "the two backends must print the same");
}

fn store_mints(env: &[(&str, &str)]) -> usize {
    let mut cmd = loft();
    cmd.arg("--interpret")
        .arg(cells())
        .env("LOFT_TRACE_DB", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("OpDatabase"))
        .count()
}

#[test]
fn the_image_changes_no_mint() {
    // The image is how a record is FILLED, not whether it is minted: the store census
    // must read the same under both switch states.
    assert_eq!(
        store_mints(&[]),
        store_mints(OFF),
        "store mints (image, walk)"
    );
}

/// How many times the run USED `Mix`'s image (`LOFT_TRACE_PREFILL=1`, one line per use).
fn image_uses(mode: &str) -> usize {
    let mut cmd = loft();
    cmd.arg(mode).arg(cells()).env("LOFT_TRACE_PREFILL", "1");
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.starts_with("[prefill] `Mix`: image used"))
        .count()
}

#[test]
fn the_cells_use_the_image_on_both_backends() {
    // A cell that only CAPTURES the image proves nothing about it — and on `--native`
    // every literal is a complete-write mint that never prefills, so c1–c10 capture
    // `Mix`'s image there and never use it; c11's re-activated caller is what makes the
    // native half of the verify pin above non-vacuous.  Both backends must USE the image.
    let interpret = image_uses("--interpret");
    let native = image_uses("--native");
    assert!(
        interpret > 0 && native > 0,
        "image uses of `Mix`: interpret {interpret}, native {native}"
    );
}
