// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DecodeView`'s structure clause — which roots read their decoded value at its
//! position, and which functions are written again as positional twins, PINNED on the cells
//! (`tests/scripts/a-decoded-value-is-read-at-its-position.loft`).  The cells' values hold
//! either way, so only this pin sees an admission move; it reads the C (`--native-emit`).  The
//! plan is empty in a build that ships the live tier, so the cells run here on the release
//! build, with the rule on and off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-decoded-value-is-read-at-its-position.loft";

/// The roots read in their positional form: each calls a positional twin.  `each` (a value
/// read in a loop), `two` (one value handed on twice) and `keep` (a value returned) keep the
/// built form.
const ROOTS: &[&str] = &[
    "n_checked",
    "n_deep",
    "n_find",
    "n_flag",
    "n_flag_num",
    "n_lookup",
    "n_num",
    "n_status",
    "n_total",
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

/// The release build's C for the cells: the positional roots, the `__ps` twins and the number
/// of walk twins.
fn emitted(env: &[(&str, &str)], tag: &str) -> (Vec<String>, Vec<String>, usize) {
    let out = std::env::temp_dir().join(format!(
        "loft_decode_structure_{tag}_{}.c",
        std::process::id()
    ));
    let mut cmd = loft();
    cmd.arg("--ship")
        .arg("--native")
        .arg("--native-emit")
        .arg(&out)
        .arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft --native-emit");
    let text = loft::file_access::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "no C emitted ({e}; exit {:?}): {}",
            res.status,
            String::from_utf8_lossy(&res.stderr)
        )
    });
    let _ = loft::file_access::remove_file(&out);
    let mut roots = Vec::new();
    let mut twins = Vec::new();
    let mut walks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        // A function's definition: `static … n_name(loft_cx *cx, …) {`; its body is `n_name__fr`.
        if line.ends_with('{')
            && !line.starts_with(' ')
            && let Some((head, _)) = line.split_once("(loft_cx")
            && let Some(name) = head.rsplit(' ').next()
            && name.starts_with("n_")
        {
            let name = name.strip_suffix("__fr").unwrap_or(name);
            current = None;
            if let Some(base) = name.strip_suffix("__ps") {
                twins.push(base.to_string());
            } else if name.contains("__psw") {
                walks.push(name.to_string());
            } else if !name.contains("__") {
                current = Some(name.to_string());
            }
        } else if line.contains("__ps(cx")
            && let Some(name) = current.take()
        {
            roots.push(name);
        }
    }
    roots.sort();
    roots.dedup();
    twins.sort();
    twins.dedup();
    walks.sort();
    walks.dedup();
    (roots, twins, walks.len())
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
            .arg("--ship")
            .arg(cells())
            .env(env.0, env.1)
            .output()
            .expect("spawn loft --ship");
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
