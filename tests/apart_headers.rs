// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Header`'s apart clause — which loops keep a header the gate's whole-loop verdict
//! would have dropped, PINNED on the cells
//! (`tests/scripts/a-vector-only-read-keeps-its-header-beside-a-writer.loft`).  The cells'
//! values hold either way, so only this pin sees an admission move; the header verifier
//! runs here too, since it is the one instrument that sees a header kept for a vector the
//! loop can reach.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-vector-only-read-keeps-its-header-beside-a-writer.loft";

/// Per function: the vectors some loop of it keeps apart (the clause ON; OFF: none).
const EXPECTED: &[(&str, &[&str])] = &[
    ("n_c1", &["v"]),       // scalar elements beside a growing callee
    ("n_c2", &["names"]),   // text elements, the table of stores growing
    ("n_c3", &["items"]),   // record elements through a local
    ("n_c4w", &["v"]),      // a work buffer the caller lends
    ("n_c5", &[]),          // handed to the writer
    ("n_c6", &[]),          // captured by a closure the loop calls
    ("n_c7", &[]),          // an element handed to a writer
    ("n_c8", &[]),          // appended to by the loop
    ("n_c9", &[]),          // declared inside the loop
    ("n_steps", &[]),       // a generator
    ("n_c11", &["a", "b"]), // the reads in a nested loop
    ("n_c12", &[]),         // an exit inside the loop releases the store
    ("n_c13", &["v"]),      // the writer is the function itself
    ("n_c14", &["v"]),      // the writer is called through a function value
];

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_APART_HEADERS")
        .env_remove("LOFT_HOIST_VERIFY");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The trace of an emission of the cells: every `keeps … apart` line.
fn kept(env: &[(&str, &str)], tag: &str) -> Vec<String> {
    let out = std::env::temp_dir().join(format!("loft_apart_headers_{tag}.rs"));
    let mut cmd = loft();
    cmd.arg("--native-rust")
        .arg("--native-emit")
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
    let _ = loft::file_access::remove_file(&out);
    String::from_utf8_lossy(&res.stderr)
        .lines()
        .filter(|l| l.contains("apart from what declined it"))
        .map(str::to_string)
        .collect()
}

/// The vectors the trace says `name` keeps, each once, sorted.
fn kept_in(lines: &[String], name: &str) -> Vec<String> {
    let head = format!("hoist: {name} loop ");
    let mut out: Vec<String> = Vec::new();
    for l in lines.iter().filter(|l| l.starts_with(&head)) {
        let list = l
            .rsplit_once(": [")
            .map_or("", |(_, r)| r.trim_end_matches(']'));
        for v in list.split(", ").map(|v| v.trim_matches('"')) {
            if !v.is_empty() && !out.iter().any(|x| x == v) {
                out.push(v.to_string());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_vector_only_read_keeps_its_header_and_a_reachable_one_does_not() {
    let lines = kept(&[], "on");
    for (name, expected) in EXPECTED {
        assert_eq!(
            kept_in(&lines, name),
            expected.to_vec(),
            "{name}: the vectors kept apart — the cell's admission moved"
        );
    }
    for l in &lines {
        let name = l
            .strip_prefix("hoist: ")
            .and_then(|r| r.split_once(' '))
            .map_or("", |(n, _)| n);
        assert!(
            EXPECTED.iter().any(|(n, _)| *n == name),
            "an unpinned function keeps a header apart: {l}"
        );
    }
}

#[test]
fn the_switch_keeps_no_header_apart() {
    let lines = kept(&[("LOFT_NO_APART_HEADERS", "1")], "off");
    assert!(
        lines.is_empty(),
        "LOFT_NO_APART_HEADERS=1 must keep none:\n{lines:?}"
    );
}

/// Every read re-derives its header and base (`LOFT_HOIST_VERIFY=1`): a header kept for a
/// vector the loop moved or grew panics here, where the cells' values could agree by luck.
#[test]
fn the_verifier_finds_no_stale_header() {
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
        out.status.success() && stdout.contains("done"),
        "the cells failed under the header verifier (exit {:?}):\n{stdout}\n{stderr}",
        out.status
    );
}
