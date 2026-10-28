// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-StreamWalk` — a vector a function builds and its caller walks once is streamed: the
//! builder's twin hands each element to the walk's body at the append that would have stored it.
//!
//! The cell corpus (`tests/scripts/a-walk-of-a-built-vector-streams-its-elements.loft`) holds
//! the VALUES, hand-computed; the interpreter never streams, so its agreement is evidence.  This
//! pins WHICH walks stream on the release build — every admitted cell, no declined one — that
//! `LOFT_NO_STREAM_WALK=1` streams none, and that the cells hold on the release build both ways.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-walk-of-a-built-vector-streams-its-elements.loft";

/// `(consumer, walks streamed)` — the predictions beside the cells.
const EXPECTED: &[(&str, usize)] = &[
    ("n_c1", 1),  // the element bound, its text and number read
    ("n_c2", 1),  // the element read in place
    ("n_c3", 1),  // a float field
    ("n_c4", 2),  // texts in order, and the builder's early return
    ("n_c5", 1),  // two append sites
    ("n_c7", 1),  // a statement between that the call does not read
    ("n_c8", 0),  // a statement between writes what the call reads
    ("n_c9", 1),  // the body appends to its own vector
    ("n_c10", 0), // the body writes the vector the builder reads
    ("n_c11", 1), // a function value with a quiet target
    ("n_c12", 0), // a call that writes what its caller handed it
    ("n_c13", 0), // `break` leaves the walk
    ("n_c14", 0), // the vector is read after the walk
    ("n_c15", 0), // the body reads the index
    ("n_c16", 2), // a walk inside a walk
    ("n_c17", 2), // a formatted text; one walk reads the number, one the text
    ("n_c18", 2), // a field an append fills with its default
    ("n_c19", 0), // the builder writes what its caller handed it
    ("n_c20", 0), // the body writes the local the builder reads
    ("n_c21", 0), // a statement between writes a field of what the call reads
];

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env_remove("LOFT_NO_STREAM_WALK")
        .env_remove("LOFT_DEV_SOFT_HALT");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The walks each consumer streams, read off the release build's emission.
fn streamed(env: &[(&str, &str)], tag: &str) -> Vec<(String, usize)> {
    let out =
        std::env::temp_dir().join(format!("loft_stream_walk_{tag}_{}.rs", std::process::id()));
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
    let mut counts: Vec<(String, usize)> = EXPECTED
        .iter()
        .map(|(n, _)| ((*n).to_string(), 0))
        .collect();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some((name, _)) = rest.split_once(['(', '<'])
        {
            // A ranged variant (`__rg`) is the same body emitted again: counted once.
            current = counts.iter().position(|(n, _)| n == name);
        } else if line.contains("//@FR-R-StreamWalk walk")
            && let Some(i) = current
        {
            counts[i].1 += 1;
        }
    }
    counts
}

#[test]
fn the_admitted_walks_stream_and_the_declined_do_not() {
    let got = streamed(&[], "on");
    let want: Vec<(String, usize)> = EXPECTED
        .iter()
        .map(|(n, c)| ((*n).to_string(), *c))
        .collect();
    assert_eq!(got, want, "the streamed walks moved");
}

#[test]
fn the_switch_streams_nothing() {
    let got = streamed(&[("LOFT_NO_STREAM_WALK", "1")], "off");
    assert!(
        got.iter().all(|(_, c)| *c == 0),
        "LOFT_NO_STREAM_WALK=1 must stream no walk: {got:?}"
    );
}

#[test]
fn the_cells_hold_on_the_release_build() {
    for off in [false, true] {
        let mut cmd = loft();
        cmd.arg("--ship").arg(cells());
        if off {
            cmd.env("LOFT_NO_STREAM_WALK", "1");
        }
        let out = cmd.output().expect("spawn loft --ship");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.trim_end().ends_with("c21 9"),
            "the cells failed with the rule {} (exit {:?}):\n{stdout}\n{}",
            if off { "off" } else { "on" },
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
