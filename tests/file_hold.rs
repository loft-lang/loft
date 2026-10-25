// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-FileHold` — which loops read a file through a held cursor, PINNED on the cells
//! (`tests/scripts/a-file-read-loop-holds-its-cursor.loft`).  The cells' values hold either
//! way, so only this pin sees an admission move.  A reader behind a helper is held where the
//! helper is inlined, which the release build does and plain `--native` does not, so this is
//! also where the cells run on the release build, under the switch and under the verifier.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-file-read-loop-holds-its-cursor.loft";

/// Per function: the loops that hold a cursor on the release build (the rule OFF: none).
const EXPECTED: &[(&str, usize)] = &[
    ("n_read_all", 1),       // h1: the reader behind a helper
    ("n_h2", 1),             // the read written in the loop
    ("n_h3", 1),             // left by a `break`
    ("n_first_over", 1),     // left by a `return`
    ("n_h5", 1),             // reads past the end
    ("n_h6", 0),             // `f#next` read in the loop
    ("n_h7", 0),             // a seek in the loop
    ("n_read_and_ask", 0),   // a second name for the file
    ("n_h9", 0),             // a call handed the file
    ("n_h10", 1),            // open before the loop
    ("n_h11", 6),            // each width, sign and byte order
    ("n_h12", 0),            // two files
    ("n_h13", 1),            // the inner loop; the outer asks for the cursor
    ("n_h15", 0),            // a structure with a file in it, written in the loop
    ("n_h16", 1),            // two widths
    ("n_h17", 2),            // loops that run no pass
    ("n_h18", 1),            // the helper's value masked
    ("n_h20", 1),            // a `continue`
    ("n_h23", 1),            // the inner loop; the outer rebinds the variable
    ("n_own_first_over", 0), // h24: a `return` in the loop releases the file
    ("NRowSumsGen", 1),      // h25: a generator's inner loop, between two yields
];

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env_remove("LOFT_NO_FILE_HOLD")
        .env_remove("LOFT_NO_INLINE_READER")
        .env_remove("LOFT_NO_TYPED_READ")
        .env_remove("LOFT_HOIST_VERIFY");
    cmd
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

/// The release build's emission of the cells: per function, how many guards it binds.
fn holds(env: &[(&str, &str)], tag: &str) -> Vec<(String, usize)> {
    let out = std::env::temp_dir().join(format!("loft_file_hold_{tag}.rs"));
    let mut cmd = loft();
    cmd.arg("--ship")
        .arg("--native-emit")
        .arg(&out)
        .arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let res = cmd.output().expect("spawn loft --native-emit");
    let src = loft::file_access::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "no Rust emitted ({e}; exit {:?}): {}",
            res.status,
            String::from_utf8_lossy(&res.stderr)
        )
    });
    let _ = loft::file_access::remove_file(&out);
    let mut found: Vec<(String, usize)> = Vec::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some((name, _)) = rest.split_once('(')
        {
            found.push((name.to_string(), 0));
        } else if let Some(rest) = line.strip_prefix("impl ")
            && let Some((_, name)) = rest.rsplit_once(" for ")
        {
            // A generator's body is its coroutine's `resume`.
            found.push((name.trim_end_matches(" {").to_string(), 0));
        } else if line.contains("codegen_runtime::FileCursor::<")
            && let Some(last) = found.last_mut()
        {
            last.1 += 1;
        }
    }
    found.retain(|(_, n)| *n > 0);
    found
}

#[test]
fn a_loop_that_alone_sees_its_file_holds_the_cursor() {
    let found = holds(&[], "on");
    for (name, expected) in EXPECTED {
        let n = found.iter().find(|(f, _)| f == name).map_or(0, |(_, n)| *n);
        assert_eq!(
            n, *expected,
            "{name}: the cursors held — the cell's admission moved"
        );
    }
    for (name, n) in &found {
        assert!(
            EXPECTED.iter().any(|(f, _)| f == name),
            "an unpinned function holds {n} file cursor(s): {name}"
        );
    }
}

#[test]
fn the_switch_holds_no_cursor() {
    let found = holds(&[("LOFT_NO_FILE_HOLD", "1")], "off");
    assert!(
        found.is_empty(),
        "LOFT_NO_FILE_HOLD=1 must hold none:\n{found:?}"
    );
}

/// Without the reader clause the helper stays a call, and the loop that calls it holds
/// nothing: the file is handed to a call.
#[test]
fn a_reader_left_behind_its_call_is_not_held() {
    let found = holds(&[("LOFT_NO_INLINE_READER", "1")], "call");
    for name in ["n_read_all", "n_h18"] {
        assert!(
            !found.iter().any(|(f, _)| f == name),
            "{name} holds a cursor for a file it hands to a call:\n{found:?}"
        );
    }
    assert!(
        found.iter().any(|(f, _)| f == "n_h2"),
        "a read written in the loop is held whatever the helper does:\n{found:?}"
    );
}

/// The cells on the release build: the rule on, off, and under the verifier, which asserts
/// at every read that the file's record still says what it said when the hold began.
#[test]
fn the_cells_hold_on_the_release_build() {
    for env in [
        ("LOFT_TIMEOUT", "300"),
        ("LOFT_NO_FILE_HOLD", "1"),
        ("LOFT_NO_INLINE_READER", "1"),
        ("LOFT_HOIST_VERIFY", "1"),
    ] {
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
