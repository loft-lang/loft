// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InPlace`'s read clause — the EMISSION pins.  `f#read(n) as <scalar>` writes the
//! scalar local and the File record's cursor fields in place, so a push loop whose body
//! reads a file keeps its vector header.  `LOFT_NO_WRITE_HOIST=1` turns the whole in-place
//! tier off.  The cell corpus (`tests/scripts/158-scalar-file-read.loft`) says the VALUES
//! hold on both backends and under `LOFT_HOIST_VERIFY=1`; this pins what is emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-scalar-file-read.loft";

/// The functions whose loop holds a header with the clause ON; none of them does OFF.  r4
/// (a text read) and r5 (a vector read) are writers either way and hold none.
const HOLDS: [&str; 4] = ["n_read_all", "n_r2", "n_r3", "n_r6"];
const WRITERS: [&str; 2] = ["n_r4", "n_r5"];

/// Per function of the emitted Rust, how many hoist blocks it opens.
fn hoist_blocks(tag: &str, env: &[(&str, &str)]) -> Vec<(String, usize)> {
    let out = std::env::temp_dir().join(format!(
        "loft_scalar_file_read_{}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args([
        "--native-emit",
        out.to_str().unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(CELLS)
            .to_str()
            .unwrap(),
    ])
    .env("LOFT_TIMEOUT", "300")
    .env_remove("LOFT_NO_WRITE_HOIST");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft");
    let src = fa::read_to_string(&out).unwrap_or_else(|e| {
        panic!(
            "no emission at {}: {e}\n{}",
            out.display(),
            String::from_utf8_lossy(&status.stderr)
        )
    });
    let _ = fa::remove_file(&out);
    let mut rows: Vec<(String, usize)> = Vec::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("fn ") {
            let name = rest.split('(').next().unwrap_or("").to_string();
            rows.push((name, 0));
        } else if line.contains("loop-invariant vector headers")
            && let Some(last) = rows.last_mut()
        {
            last.1 += 1;
        }
    }
    rows
}

fn blocks(rows: &[(String, usize)], name: &str) -> usize {
    rows.iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} not emitted"))
        .1
}

#[test]
fn a_push_loop_that_reads_a_scalar_from_a_file_holds_its_header() {
    let on = hoist_blocks("on", &[]);
    let off = hoist_blocks("off", &[("LOFT_NO_WRITE_HOIST", "1")]);
    for name in HOLDS {
        assert_eq!(
            blocks(&on, name),
            1,
            "{name}: the read clause should hold the header"
        );
        assert_eq!(
            blocks(&off, name),
            0,
            "{name}: LOFT_NO_WRITE_HOIST=1 holds nothing"
        );
    }
    for name in WRITERS {
        assert_eq!(
            blocks(&on, name),
            0,
            "{name}: a text or vector read stays a writer"
        );
    }
}
