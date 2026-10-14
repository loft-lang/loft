// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Apart` instance 2 — which functions have an apart twin and which bodies hold apart
//! values, PINNED on the cells (`tests/scripts/a-record-of-fixed-vectors-travels-as-a-value.loft`).
//! The cells' values hold either way, so only this pin sees an admission move.  The plan is
//! empty in a build that ships the live tier, so the cells run here on the release build, with
//! the rule on and off; and an index out of range must be REPORTED as the store path reports
//! it, which the halt switch makes visible.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-record-of-fixed-vectors-travels-as-a-value.loft";

/// The functions with a twin (`n_<f>__apv`).
const TWINS: &[&str] = &[
    "n_cube",
    "n_e2",
    "n_e4",
    "n_first_over",
    "n_ident",
    "n_last",
    "n_mk",
    "n_mul",
    "n_pp",
    "n_pr",
    "n_rot",
    "n_setneg",
    "n_swap",
];

/// The ordinary bodies that hold an apart value.  `n_last` and `n_swap` only build their
/// result from a store parameter and return it, which would pay the conversion for nothing.
const REGIONS: &[&str] = &[
    "n_cube",
    "n_e2",
    "n_e4",
    "n_first_over",
    "n_mul",
    "n_pp",
    "n_setneg",
];

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
    cmd.arg("--native-release")
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
fn the_twins_and_regions_are_where_the_cells_say() {
    let (twins, regions) = emitted(&cells(), &[], "on");
    assert_eq!(twins, TWINS, "the twins moved");
    assert_eq!(regions, REGIONS, "the regions moved");
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
            .arg("--native-release")
            .arg(cells())
            .env(env.0, env.1)
            .output()
            .expect("spawn loft --native-release");
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

/// A read out of range in a twin reports the error the store path reports, and answers null.
#[test]
fn an_index_out_of_range_reports_as_the_store_path_does() {
    let src = std::env::temp_dir().join(format!("loft_apart_oob_{}.loft", std::process::id()));
    loft::file_access::write(
        &src,
        "struct M2 { m: vector<float> }\n\
         fn mk(a: float) -> M2 { M2 { m: [a, a + 1.0, a + 2.0, a + 3.0] } }\n\
         fn oob(x: const M2) -> M2 {\n  r = M2 { m: [0.0, 0.0, 0.0, 0.0] };\n  r.m[0] = x.m[7];\n  r.m[1] = x.m[-9];\n  r.m[9] = 1.0;\n  r\n}\n\
         fn op() -> M2 { a = mk(1.0); b = oob(a); b }\n\
         fn main() { z = op(); println(\"{z.m}\"); }\n",
    )
    .expect("write the probe");
    let (twins, _) = emitted(&src, &[], "oob");
    assert!(
        twins.iter().any(|t| t == "n_oob"),
        "the probe's reads are no twin's: {twins:?}"
    );
    let run = |apart: bool| {
        let mut cmd = loft();
        cmd.arg("--native-release")
            .arg(&src)
            .env("LOFT_DEV_SOFT_HALT", "1");
        if !apart {
            cmd.env("LOFT_NO_APART", "1");
        }
        let out = cmd.output().expect("spawn loft --native-release");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (on_out, on_err) = run(true);
    let (off_out, off_err) = run(false);
    let _ = loft::file_access::remove_file(&src);
    assert_eq!(
        on_out.trim(),
        "[null,null,0,0]",
        "the values out of range: {on_err}"
    );
    assert_eq!(on_out, off_out, "the two forms answer differently");
    let reports = |e: &str| -> Vec<String> {
        e.lines()
            .filter(|l| l.starts_with("soft-halt:"))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        reports(&on_err),
        [
            "soft-halt: index 7 out of bounds for length 4",
            "soft-halt: negative index -9",
            "soft-halt: index 9 out of bounds for length 4",
        ],
        "the reports: {on_err}"
    );
    assert_eq!(
        reports(&on_err),
        reports(&off_err),
        "the two forms report differently"
    );
}
