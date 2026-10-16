// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Header`'s apart clause, its PARAMETER half — which loops keep a parameter's vector
//! header, PINNED on the cells
//! (`tests/scripts/a-parameters-vector-holds-its-header-where-the-buffer-is-apart.loft`).
//! The cells' values hold whichever loops keep it, so only this pin sees an admission move;
//! and the header verifier runs here, the one instrument that sees a header kept for a
//! vector the loop can reach when the stale bytes happen to read right.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "tests/scripts/a-parameters-vector-holds-its-header-where-the-buffer-is-apart.loft";

/// Per function: the vectors some loop of it keeps apart (the clause ON).
const EXPECTED: &[(&str, &[&str])] = &[
    ("n_runs_of", &["d"]), // the versioned copy: the return buffer tested apart from `d`
    ("n_mirror", &[]),     // the store grown through another parameter
    ("n_drain", &[]),      // the vector emptied through another parameter
];

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_APART_HEADERS")
        .env_remove("LOFT_NO_APART_PARAM")
        .env_remove("LOFT_HOIST_VERIFY");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The functions of the cells' emission and the vectors their loops keep apart.
fn kept(env: &[(&str, &str)], tag: &str) -> Vec<(String, Vec<String>)> {
    let out = std::env::temp_dir().join(format!("loft_apart_param_{tag}.rs"));
    let mut cmd = loft();
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(cells())
        .env("LOFT_TRACE_HOIST_DECLINE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft --native-emit");
    assert!(
        loft::file_access::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        res.status,
        String::from_utf8_lossy(&res.stderr)
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for line in stderr.lines() {
        // `hoist: <fn> loop <n> keeps <k> header(s) apart from what declined it: ["a", "b"]`
        let Some(rest) = line.strip_prefix("hoist: ") else {
            continue;
        };
        let Some((func, tail)) = rest.split_once(" loop ") else {
            continue;
        };
        let Some((_, list)) = tail.split_once("declined it: ") else {
            continue;
        };
        let names: Vec<String> = list
            .trim_matches(|c| c == '[' || c == ']')
            .split(", ")
            .filter(|n| !n.is_empty())
            .map(|n| n.trim_matches('"').to_string())
            .collect();
        match found.iter_mut().find(|(f, _)| f == func) {
            Some((_, all)) => {
                for n in names {
                    if !all.contains(&n) {
                        all.push(n);
                    }
                }
            }
            None => found.push((func.to_string(), names)),
        }
    }
    found
}

fn of<'a>(found: &'a [(String, Vec<String>)], func: &str) -> &'a [String] {
    found
        .iter()
        .find(|(f, _)| f == func)
        .map_or(&[][..], |(_, v)| v.as_slice())
}

#[test]
fn each_loop_keeps_the_parameter_headers_the_clause_admits() {
    let found = kept(&[], "on");
    for (func, want) in EXPECTED {
        let got = of(&found, func);
        assert_eq!(
            got, *want,
            "{func}: the loops keep {got:?} apart where the clause admits {want:?}\nall: {found:?}"
        );
    }
}

#[test]
fn the_switch_keeps_no_parameter_header() {
    let found = kept(&[("LOFT_NO_APART_PARAM", "1")], "off");
    for (func, _) in EXPECTED {
        assert!(
            of(&found, func).is_empty(),
            "LOFT_NO_APART_PARAM=1 must keep no parameter's header in {func}: {found:?}"
        );
    }
}

/// Every read re-derives its header and base (`LOFT_HOIST_VERIFY=1`): a header kept for a
/// vector the loop moved or emptied panics here, where the cells' values could agree by luck.
#[test]
fn the_verifier_finds_no_stale_parameter_header() {
    let out = loft()
        .arg("--native")
        .arg(cells())
        .env("LOFT_HOIST_VERIFY", "1")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .output()
        .expect("spawn loft --native");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("ok"),
        "the cells failed under the header verifier (exit {:?}):\n{stdout}\n{stderr}",
        out.status
    );
}
