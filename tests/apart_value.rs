// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: reads the Rust writer's apart-value reports — its `--native` runs as `--native-rust`.
//! `@FR-R-Apart` instance 2 — which functions have an apart twin and which bodies hold apart
//! values, PINNED on the cells (`tests/scripts/a-record-of-fixed-vectors-travels-as-a-value.loft`).
//! The cells' values hold either way, so only this pin sees an admission move.  The plan is
//! empty in a build that ships the live tier, so the cells run here on the release build, with
//! the rule on and off; and an index out of range must be REPORTED as the store path reports
//! it, which the halt switch makes visible.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-record-of-fixed-vectors-travels-as-a-value.loft";

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env_remove("LOFT_NO_APART")
        .env_remove("LOFT_DEV_SOFT_HALT");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The release build's emission of `src`: the twins, and the ordinary bodies naming an
/// apart local.
fn emitted(src: &Path, env: &[(&str, &str)], tag: &str) -> (Vec<String>, Vec<String>) {
    let out = std::env::temp_dir().join(format!("loft_apart_value_{tag}.rs"));
    let mut cmd = loft();
    cmd.arg("--ship")
        .arg("--native")
        .arg("--native-emit")
        .arg(&out)
        .arg(src);
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
    let mut twins = Vec::new();
    let mut regions = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some((name, _)) = rest.split_once('(')
        {
            if let Some(base) = name.strip_suffix("__apv") {
                twins.push(base.to_string());
                current = None;
            } else {
                // A ranged variant (`__rg`) is the same body emitted again.
                current = Some(name.trim_end_matches("__rg").to_string());
            }
        } else if line.contains("let mut __ap_")
            && let Some(name) = current.take()
        {
            regions.push(name);
        }
    }
    twins.sort();
    regions.sort();
    regions.dedup();
    (twins, regions)
}

#[test]
fn the_switch_leaves_every_record_in_the_store() {
    let (twins, regions) = emitted(&cells(), &[("LOFT_NO_APART", "1")], "off");
    assert!(
        twins.is_empty() && regions.is_empty(),
        "LOFT_NO_APART=1 must emit no apart value: {twins:?} {regions:?}"
    );
}

#[test]
fn the_cells_hold_on_the_release_build() {
    for env in [("LOFT_TIMEOUT", "300"), ("LOFT_NO_APART", "1")] {
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

const VECTOR_CELLS: &str = "tests/scripts/a-local-float-vector-lives-in-an-inline-buffer.loft";

#[test]
fn the_vector_cells_hold_with_the_rule_off() {
    for tier in ["--native", "--ship"] {
        let out = loft()
            .env("LOFT_NO_APART_VECTOR", "1")
            .arg(tier)
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTOR_CELLS))
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.trim_end().ends_with("ok"),
            "{tier} with the rule off:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
