// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RangedCall`'s variable clause — the EMISSION pins.  A window walk whose bounds are
//! integer parameters guards them once at the block's entry, and its guarded copy calls the
//! ranged variants of its helpers, reached through a tuple the walk takes apart (`@FR-R-Range`'s
//! tuple elements).  A tuple written element by element carries no element fact.  The cell
//! corpus (`tests/scripts/a-window-walk-guards-its-bounds-once-and-calls-plain-helpers.loft`)
//! says the VALUES hold; this pins what is emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "tests/scripts/a-window-walk-guards-its-bounds-once-and-calls-plain-helpers.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_rcvars_{}_{tag}.rs", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-rust")
        .arg("--native-emit")
        .arg(&out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_RANGED_CALLS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft");
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

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

#[test]
fn a_window_walk_guards_its_parameters_and_calls_the_ranged_helpers() {
    let rust = emit("on", &[]);
    let walk = body(&rust, "n_walk");
    for v in ["var_q0", "var_r0", "var_w", "var_h"] {
        assert!(
            walk.contains(&format!("({v}).unsigned_abs() <= 1048576u64")),
            "n_walk does not guard {v}:\n{walk}"
        );
    }
    assert!(
        walk.contains("n_score__rg("),
        "the guarded copy does not call the ranged score:\n{walk}"
    );
    assert!(
        walk.contains("n_step__rg("),
        "the guarded copy does not call the ranged step:\n{walk}"
    );
}

#[test]
fn a_tuple_written_by_element_keeps_its_helper_checked() {
    let rust = emit("put", &[]);
    let walk_put = body(&rust, "n_walk_put");
    assert!(
        !walk_put.contains("n_score__rg("),
        "`t.0 = big` must leave t's element unranged, so score stays checked:\n{walk_put}"
    );
}

#[test]
fn switched_off_nothing_is_guarded() {
    let rust = emit("off", &[("LOFT_NO_RANGED_CALLS", "1")]);
    assert!(!body(&rust, "n_walk").contains("unsigned_abs() <= 1048576u64"));
    assert!(!rust.contains("fn n_score__rg("));
}

#[test]
fn the_variable_clause_switched_off_guards_no_variable() {
    let rust = emit("vars_off", &[("LOFT_NO_RANGED_CALL_VARS", "1")]);
    let walk = body(&rust, "n_walk");
    assert!(!walk.contains("(var_q0).unsigned_abs()"), "{walk}");
    assert!(!walk.contains("n_score__rg("), "{walk}");
}
