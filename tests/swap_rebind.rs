// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-SwapRebind`, PINNED on its cells (`tests/scripts/a-rebind-exchanges-into-a-destination-it-does-not-reset.loft`):
//! every cell's rebind is emitted as `OpRebindRecord` on `--native`, the cells answer the same
//! with the switch on and off, and no store is left behind — a released store the exchange
//! kept instead of freeing reads as a leak here and nowhere else.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-rebind-exchanges-into-a-destination-it-does-not-reset.loft";

/// `(cell, OpRebindRecord sites)` in the emitted Rust.
const SITES: &[(&str, usize)] = &[
    ("n_r1", 1),
    ("n_r2", 1),
    ("n_r3", 1),
    ("n_r4", 2),
    ("n_r5", 1),
    ("n_r6", 1),
    ("n_r7", 1),
    ("n_r8", 1),
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_SWAP_REBIND")
        .env_remove("LOFT_NO_STORE_SWAP");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn every_cell_rebinds_through_the_exchange() {
    let rs = std::env::temp_dir().join(format!("loft_swap_rebind_{}.rs", std::process::id()));
    let (_, err, ok) = loft(&["--native-emit", &rs.to_string_lossy(), &cells()], &[]);
    assert!(ok, "emit failed:\n{err}");
    let src = std::fs::read_to_string(&rs).expect("emitted source");
    let _ = std::fs::remove_file(&rs);
    for (cell, want) in SITES {
        let head = format!("fn {cell}(");
        let body = src
            .split("\nfn ")
            .find(|f| format!("fn {f}").starts_with(&head))
            .unwrap_or_else(|| panic!("{cell} not emitted"));
        let got = body.matches("OpRebindRecord(").count();
        assert_eq!(
            got, *want,
            "{cell}: {got} OpRebindRecord sites, expected {want}"
        );
    }
}

#[test]
fn the_cells_answer_the_same_and_free_every_store() {
    for env in [&[][..], &[("LOFT_NO_SWAP_REBIND", "1")][..]] {
        let mut env = env.to_vec();
        env.push(("LOFT_NATIVE_LEAK_CHECK", "1"));
        let (out, err, ok) = loft(&["--native", &cells()], &env);
        assert!(ok && out.contains("done"), "{env:?}:\n{out}\n{err}");
        assert!(
            !err.contains("not freed"),
            "{env:?} left stores behind:\n{err}"
        );
    }
}
