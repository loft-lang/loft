// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr`'s index clause and `@FR-H-ClearRelease`'s scalar clear — the EMISSION pins.
//! `r = v[IDX]?` over a vector whose header and base the frame holds takes its address from
//! the held base by index, and a buffer of scalars is cleared by the plain reset.
//! `LOFT_NO_INDEX_RECPTR=1` removes every index address and leaves the clears alone.  The
//! cells (`tests/scripts/a-record-read-by-index-from-a-held-vector-is-addressed-off-its-base.loft`)
//! say the VALUES hold on both backends and under every switch; this pins what is emitted.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "tests/scripts/a-record-read-by-index-from-a-held-vector-is-addressed-off-its-base.loft";

/// Per function: `(addresses from the held base by index, plain scalar clears)`.
const EXPECTED: &[(&str, usize, usize)] = &[
    // The polygon edge walk: `p[i]?` and `p[(i + 1) % n ?? 0]?` — any repeatable index.
    ("n_edges", 2, 0),
    ("n_past", 1, 0),
    ("n_negative", 1, 0),
    ("n_null_index", 1, 0),
    // A `&` parameter's vector holds no header in the plain function; its twin is handed one.
    ("n_bump", 0, 0),
    ("n_bump__inv", 1, 0),
    ("n_bump_local", 1, 0),
    // Two index views, and the builder's buffer cleared plainly at entry and at the bind.
    ("n_lows", 2, 2),
    ("n_or_local", 1, 2),
    // `o = w.xs[0]?` reads a vector the loop grows: no held header, no index address.
    ("n_mixed", 1, 0),
    ("n_same_source", 1, 0),
    ("n_texts", 1, 0),
    ("n_evens", 0, 2),
    // The second slice into `w` clears the first, a view, plainly.
    ("n_c10", 0, 1),
];

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let out =
        std::env::temp_dir().join(format!("loft_index_recptr_{}_{tag}.rs", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_INDEX_RECPTR");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let cells = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let status = cmd
        .args([
            "--native-emit",
            out.to_str().unwrap(),
            cells.to_str().unwrap(),
        ])
        .output()
        .expect("spawn loft");
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
    rust
}

fn counts(rust: &str) -> HashMap<String, (usize, usize)> {
    let mut map: HashMap<String, (usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += line.matches("from the held base by index").count();
        row.1 += line.matches("stores.clear_vector_plain(").count();
    }
    map
}

#[test]
fn index_views_take_their_address_from_the_held_base() {
    let got = counts(&emit("on", &[]));
    for (name, by_index, plain) in EXPECTED {
        assert_eq!(
            got.get(*name).copied(),
            Some((*by_index, *plain)),
            "{name}: (addresses by index, plain clears)"
        );
    }
}

#[test]
fn the_switch_removes_every_index_address_and_no_clear() {
    let got = counts(&emit("off", &[("LOFT_NO_INDEX_RECPTR", "1")]));
    for (name, _, plain) in EXPECTED {
        assert_eq!(
            got.get(*name).copied(),
            Some((0, *plain)),
            "{name} under LOFT_NO_INDEX_RECPTR=1"
        );
    }
}
