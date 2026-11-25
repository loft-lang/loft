// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 rust-writer: counts the Rust writer's adoptions — its `--native` runs as `--native-rust`.
//! @PLN164 B1 (`@FR-O-Move`) — ADOPT AT FIRST BIND: a plain local first-bound from a callee
//! that returns the local it promoted onto its hidden buffer (`fn mk() -> P { o = P { … }; …;
//! o }`, deps `["o"]`) adopts the store the callee minted instead of minting a second one
//! and deep-copying it.  The cell corpus
//! (`plans/164-activation-arena/bytecode-comparisons/B1-adopt-first-bind-cells.loft`) says
//! the VALUES hold on both backends; this pins the PROTOCOL — the interpreter binds by
//! `PutRef` and native by the plain assignment, the free is guarded by identity against the
//! call's buffer, that buffer is NOT pooled at function entry, a bound local that is itself a
//! return buffer keeps its in-place copy, and `LOFT_NO_ADOPT_FIRST_BIND=1` restores the copy
//! — and the store census the change buys.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/B1-adopt-first-bind-cells.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // B1 on its own: B1b (`tests/adopt_buffer_reuse.rs`) pools the buffer this file pins as null.
    // The tests run in parallel on one source file, and its program cache is shared: an
    // `introspect` that read another process's half-written entry listed a truncated program.
    cmd.env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_ADOPT_BUFFER_REUSE", "1")
        .env_remove("LOFT_NO_ADOPT_FIRST_BIND");
    cmd
}

fn introspect(env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect").arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The interpreter's IR + bytecode listing of one function.
fn ir_section<'a>(listing: &'a str, name: &str) -> &'a str {
    let start = listing
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} is not in the introspect listing"));
    let rest = &listing[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

const OFF: &[(&str, &str)] = &[("LOFT_NO_ADOPT_FIRST_BIND", "1")];

#[test]
fn the_interpreter_binds_by_put_ref() {
    let on = introspect(&[]);
    let c1 = ir_section(&on, "n_c1");
    assert!(
        c1.contains("b(5):ref(P) = n_mk_loc(i(3), __ref_2(1));"),
        "c1 IR: a direct bind"
    );
    assert!(
        c1.contains("OpFreeRefIfDistinct(b(5), __ref_2(1));"),
        "c1 IR: the guarded free"
    );
    assert!(
        !c1.contains("OpDatabase(__ref_2(1)"),
        "c1 IR: the buffer is not pooled"
    );
    assert!(
        !c1.contains("CopyRefOrNull"),
        "c1 bytecode: no deep copy for `b`"
    );
    let off = introspect(OFF);
    let c1_off = ir_section(&off, "n_c1");
    assert!(
        c1_off.contains("CopyRefOrNull"),
        "c1 bytecode under the switch: the copy is back"
    );
    assert!(
        c1_off.contains("OpFreeRef(b(5));"),
        "c1 IR under the switch: the plain free"
    );
}
