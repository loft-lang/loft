// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RebindBuffer`, PINNED on its cells (`tests/scripts/a-rebind-hands-the-displaced-store-to-the-call.loft`):
//! which rebinds hand the displaced store to the call's hidden buffer, that both switch states
//! answer the same and leave no store behind on `--native`, and that strict stores see no
//! reference into a released store.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-rebind-hands-the-displaced-store-to-the-call.loft";

/// `(function, hands the store on)` in the emitted Rust.  `b3`'s rebind is a text-holding
/// record answered through a plain assignment, which this rule does not reach.  `b4`, `b7` and
/// `b8` call a function that returns an ARGUMENT on one path, through a `__ret_N = a` bind both
/// backends COPY, so every path is fresh and the store is handed on (loft#1881: a copied bind is
/// the local's own).
const SITES: &[(&str, bool)] = &[
    ("n_b1", true),
    ("n_b2", true),
    ("n_b3", false),
    ("n_b4", true),
    ("n_b5", true),
    ("n_b7", true),
    ("n_b8", true),
    ("n_b9", true),
    ("n_acc", true),
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_REBIND_BUFFER")
        .env_remove("LOFT_NO_SWAP_REBIND")
        .env_remove("LOFT_NO_REFILL_BUFFER");
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

fn body<'a>(src: &'a str, name: &str) -> &'a str {
    let head = format!("\nfn {name}(");
    let start = src
        .find(&head)
        .unwrap_or_else(|| panic!("{name} not emitted"));
    let rest = &src[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

#[test]
fn the_rebinds_that_hand_the_store_on() {
    let rs = std::env::temp_dir().join(format!("loft_rebind_buf_{}.rs", std::process::id()));
    let (_, err, ok) = loft(&["--native-emit", &rs.to_string_lossy(), &cells()], &[]);
    assert!(ok, "emit failed:\n{err}");
    let src = fa::read_to_string(&rs).expect("emitted source");
    let _ = fa::remove_file(&rs);
    for (f, hands) in SITES {
        assert_eq!(
            body(&src, f).contains(" = true; var_"),
            *hands,
            "{f}: the hand-on expected {hands}"
        );
    }
}

#[test]
fn the_cells_answer_the_same_and_free_every_store() {
    for env in [
        &[("LOFT_NATIVE_LEAK_CHECK", "1")][..],
        &[
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_NO_REBIND_BUFFER", "1"),
        ][..],
        &[("LOFT_STRICT_STORES", "1"), ("LOFT_POISON", "1")][..],
    ] {
        let (out, err, ok) = loft(&["--native", &cells()], env);
        assert!(ok, "{env:?}:\n{out}\n{err}");
        assert!(
            !err.contains("not freed"),
            "{env:?} left stores behind:\n{err}"
        );
    }
}
