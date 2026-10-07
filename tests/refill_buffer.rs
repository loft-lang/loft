// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RefillBuffer`, PINNED on its cells (`tests/scripts/a-return-buffer-refills-the-store-a-rebind-released.loft`):
//! which callees mint their return buffer with `OpDatabaseRefill` and empty its vector fields
//! in place, that the text-holding type declines, and that both switch states answer the same
//! and leave no store behind on `--native`.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-return-buffer-refills-the-store-a-rebind-released.loft";

/// `(callee, refills)` in the emitted Rust.
const CALLEES: &[(&str, bool)] = &[
    ("n_mul4", true),
    ("n_run", true),
    ("n_twice", true),
    ("n_grow", true),
    ("n_pick", true),
    ("n_named", false),
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_REFILL_BUFFER")
        .env_remove("LOFT_NO_SWAP_REBIND")
        .env_remove("LOFT_NO_REFILL_IN_PLACE");
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
fn the_refillable_builders_mint_with_the_kept_store() {
    let rs = std::env::temp_dir().join(format!("loft_refill_{}.rs", std::process::id()));
    let (_, err, ok) = loft(&["--native-emit", &rs.to_string_lossy(), &cells()], &[]);
    assert!(ok, "emit failed:\n{err}");
    let src = fa::read_to_string(&rs).expect("emitted source");
    let _ = fa::remove_file(&rs);
    for (callee, refills) in CALLEES {
        let b = body(&src, callee);
        assert_eq!(
            b.contains("OpDatabaseRefill("),
            *refills,
            "{callee}: OpDatabaseRefill expected {refills}"
        );
        assert_eq!(
            b.contains("//@FR-R-RefillBuffer"),
            *refills,
            "{callee}: the in-place vector empty expected {refills}"
        );
    }
}

#[test]
fn the_cells_answer_the_same_and_free_every_store() {
    for env in [&[][..], &[("LOFT_NO_REFILL_BUFFER", "1")][..]] {
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

/// The in-place clause's cells: every callee starts its vector field as a repeat literal.
const IN_PLACE: &str = "tests/scripts/a-refilled-repeat-literal-is-overwritten-in-place.loft";

#[test]
fn a_refilled_repeat_literal_is_overwritten_in_place() {
    let cells = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(IN_PLACE)
        .to_string_lossy()
        .into_owned();
    let rs = std::env::temp_dir().join(format!("loft_refill_ip_{}.rs", std::process::id()));
    let (_, err, ok) = loft(&["--native-emit", &rs.to_string_lossy(), &cells], &[]);
    assert!(ok, "emit failed:\n{err}");
    let src = fa::read_to_string(&rs).expect("emitted source");
    let _ = fa::remove_file(&rs);
    for callee in ["n_bump", "n_grow", "n_tri", "n_half"] {
        assert!(
            body(&src, callee).contains("//@FR-R-RefillBuffer in place"),
            "{callee}: the in-place fill expected"
        );
    }
    for env in [&[][..], &[("LOFT_NO_REFILL_IN_PLACE", "1")][..]] {
        let mut env = env.to_vec();
        env.push(("LOFT_NATIVE_LEAK_CHECK", "1"));
        let (out, err, ok) = loft(&["--native", &cells], &env);
        assert!(ok, "{env:?}:\n{out}\n{err}");
        assert!(
            !err.contains("not freed"),
            "{env:?} left stores behind:\n{err}"
        );
    }
}
