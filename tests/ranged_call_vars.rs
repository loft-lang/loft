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

/// The cell as C (`--emit-c`, the closed program the C sweep builds).
fn emit_c(tag: &str, env: &[(&str, &str)]) -> String {
    let out = std::env::temp_dir().join(format!("loft_rcvars_{}_{tag}.c", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--emit-c")
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
        "no C emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let c = fa::read_to_string(&out).expect("read the emitted C");
    let _ = fa::remove_file(&out);
    c
}

/// The C writes the same guarded copy (`opt::ranged_call::guard_plan`): the walk's bounds
/// tested once against `G` (`(uint64_t) x + G <= 2G`, a null failing it), and inside the copy
/// the helper called as its `__rg` twin (`c::RG_BIT`), whose operators the seeds prove plain.
#[test]
fn the_c_guards_the_walk_and_calls_the_ranged_twin() {
    let c = emit_c("c_on", &[]);
    assert!(
        c.contains("+ UINT64_C(1048576) <= UINT64_C(2097152)"),
        "the C writes no guarded copy of the walk"
    );
    assert!(
        c.contains("__rg("),
        "the C's guarded copy calls no ranged twin"
    );
    let off = emit_c("c_off", &[("LOFT_NO_RANGED_CALLS", "1")]);
    assert!(!off.contains("UINT64_C(1048576)") && !off.contains("__rg("));
}
