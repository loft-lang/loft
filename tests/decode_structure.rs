// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DecodeView`'s structure clause — which roots read their decoded value at its
//! position, and which functions are re-emitted as positional twins, PINNED on the cells
//! (`tests/scripts/a-decoded-value-is-read-at-its-position.loft`).  The cells' values hold
//! either way, so only this pin sees an admission move.  The plan is empty in a build that
//! ships the live tier, so the cells run here on the release build, with the rule on and off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-decoded-value-is-read-at-its-position.loft";

/// The roots read in their positional form: each calls a positional twin.  `each` (a value
/// read in a loop) and `keep` (a value returned) keep the built form.
const ROOTS: &[&str] = &[
    "n_deep", "n_find", "n_lookup", "n_num", "n_status", "n_total", "n_two",
];

/// The consumers' twins (`__ps`), the producer's shape twin among them.
const CONSUMERS: &[&str] = &[
    "n_dec",
    "n_find_in",
    "n_get",
    "n_num_of",
    "n_rd",
    "n_sum_items",
    "n_text_of",
];

/// The producer's walk twins: a map's entries and an array's items.
const WALKS: usize = 2;

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env_remove("LOFT_NO_DECODE_VIEW")
        .env_remove("LOFT_NO_DECODE_STRUCTURE");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The release build's emission of the cells: the positional roots, the `__ps` twins and the
/// number of walk twins.
fn emitted(env: &[(&str, &str)], tag: &str) -> (Vec<String>, Vec<String>, usize) {
    let out = std::env::temp_dir().join(format!(
        "loft_decode_structure_{tag}_{}.rs",
        std::process::id()
    ));
    let mut cmd = loft();
    cmd.arg("--native-release")
        .arg("--native-emit")
        .arg(&out)
        .arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft --native-emit");
    let text = loft::file_access::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "no Rust emitted ({e}; exit {:?}): {}",
            res.status,
            String::from_utf8_lossy(&res.stderr)
        )
    });
    let _ = loft::file_access::remove_file(&out);
    let mut roots = Vec::new();
    let mut twins = Vec::new();
    let mut walks = 0;
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some((name, _)) = rest.split_once(['(', '<'])
        {
            if let Some(base) = name.strip_suffix("__ps") {
                twins.push(base.to_string());
                current = None;
            } else if name.contains("__psw") {
                walks += 1;
                current = None;
            } else if name.contains("__") {
                current = None;
            } else {
                current = Some(name.to_string());
            }
        } else if line.contains("__ps(cell")
            && let Some(name) = current.take()
        {
            roots.push(name);
        }
    }
    roots.sort();
    roots.dedup();
    twins.sort();
    twins.dedup();
    (roots, twins, walks)
}

#[test]
fn the_roots_and_the_twins_are_where_the_cells_say() {
    let (roots, twins, walks) = emitted(&[], "on");
    assert_eq!(roots, ROOTS, "the positional roots moved");
    assert_eq!(twins, CONSUMERS, "the positional twins moved");
    assert_eq!(walks, WALKS, "the walk twins moved");
}

#[test]
fn the_switch_builds_every_decoded_value() {
    let (roots, twins, walks) = emitted(&[("LOFT_NO_DECODE_STRUCTURE", "1")], "off");
    assert!(
        roots.is_empty() && twins.is_empty() && walks == 0,
        "LOFT_NO_DECODE_STRUCTURE=1 must emit no positional twin: {roots:?} {twins:?} {walks}"
    );
}

#[test]
fn the_cells_hold_on_the_release_build() {
    for env in [("LOFT_TIMEOUT", "300"), ("LOFT_NO_DECODE_STRUCTURE", "1")] {
        let out = loft()
            .arg("--native-release")
            .arg(cells())
            .env(env.0, env.1)
            .output()
            .expect("spawn loft --native-release");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.trim_end().ends_with("done"),
            "the cells failed with {}={} (exit {:?}):\n{stdout}\n{stderr}",
            env.0,
            env.1,
            out.status
        );
    }
}
