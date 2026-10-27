// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DecodeView` instance 1 — which functions decode into spans of their frame, and
//! which region functions are re-emitted to read them, PINNED on the cells
//! (`tests/scripts/a-decoded-text-is-read-where-it-lies.loft`).  The cells' values hold either
//! way, so only this pin sees an admission move.  The plan is empty in a build that ships the
//! live tier, so the cells run here on the release build, with the rule on and off.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-decoded-text-is-read-where-it-lies.loft";

/// The roots: each names its frame for the length of its call.
const ROOTS: &[&str] = &[
    "n_copied",
    "n_count_unread",
    "n_first_key",
    "n_lookup",
    "n_measure",
    "n_mixed",
    "n_nested",
    "n_shout",
    "n_walk_values",
];

/// The region: every function a decoded value reaches from a root, as its `__dv` twin.
const TWINS: &[&str] = &["n_dec", "n_get", "n_read_v", "n_text_of", "t_6vector_len"];

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env_remove("LOFT_NO_DECODE_VIEW");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The release build's emission of the cells: the roots, the twins, and how many span writes
/// the twins make.
fn emitted(env: &[(&str, &str)], tag: &str) -> (Vec<String>, Vec<String>, usize) {
    let out =
        std::env::temp_dir().join(format!("loft_decode_view_{tag}_{}.rs", std::process::id()));
    let mut cmd = loft();
    cmd.arg("--ship")
        .arg("--native-rust")
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
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some((name, _)) = rest.split_once('(')
        {
            current = Some(name.to_string());
            // A twin's ranged, destination or view-return variant is the same twin.
            if let Some((base, _)) = name.split_once("__dv") {
                twins.push(base.to_string());
            }
        } else if line.contains("DvFrame::enter")
            && let Some(name) = current.take()
        {
            roots.push(name);
        }
    }
    roots.sort();
    roots.dedup();
    twins.sort();
    twins.dedup();
    let spans = text.matches("codegen_runtime::dv_span_text(").count()
        + text.matches("codegen_runtime::dv_put_span(").count();
    (roots, twins, spans)
}

#[test]
fn the_roots_and_the_region_are_where_the_cells_say() {
    let (roots, twins, spans) = emitted(&[], "on");
    assert_eq!(roots, ROOTS, "the roots moved");
    assert_eq!(twins, TWINS, "the region moved");
    assert!(spans > 0, "the twins write no span");
}

#[test]
fn the_switch_keeps_every_field_owned() {
    let (roots, twins, spans) = emitted(&[("LOFT_NO_DECODE_VIEW", "1")], "off");
    assert!(
        roots.is_empty() && twins.is_empty() && spans == 0,
        "LOFT_NO_DECODE_VIEW=1 must emit no span: {roots:?} {twins:?} {spans}"
    );
}

#[test]
fn the_cells_hold_on_the_release_build() {
    for env in [("LOFT_TIMEOUT", "300"), ("LOFT_NO_DECODE_VIEW", "1")] {
        let out = loft()
            .arg("--ship")
            .arg(cells())
            .env(env.0, env.1)
            .output()
            .expect("spawn loft --ship");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.trim_end().ends_with("ok"),
            "the cells failed with {}={} (exit {:?}):\n{stdout}\n{stderr}",
            env.0,
            env.1,
            out.status
        );
    }
}
