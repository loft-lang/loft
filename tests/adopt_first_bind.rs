// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
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
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // B1 on its own: B1b (`tests/adopt_buffer_reuse.rs`) pools the buffer this file pins as null.
    cmd.env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_ADOPT_BUFFER_REUSE", "1")
        .env_remove("LOFT_NO_ADOPT_FIRST_BIND");
    cmd
}

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("--native-emit").arg(out).arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
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

/// The emitted body of `fn <name>(` up to the next top-level `fn`.
fn fn_body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
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

fn store_mints(mode: &str, env: &[(&str, &str)]) -> usize {
    let mut cmd = loft();
    cmd.arg(mode).arg(cells()).env("LOFT_TRACE_DB", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("OpDatabase"))
        .count()
}

const OFF: &[(&str, &str)] = &[("LOFT_NO_ADOPT_FIRST_BIND", "1")];

#[test]
fn native_binds_the_minted_store_directly() {
    let out = std::env::temp_dir().join("loft_adopt_first_bind_on.rs");
    let rust = emit(&out, &[]);
    let c1 = fn_body(&rust, "n_c1");
    assert!(
        c1.contains("let mut var_b: DbRef = n_mk_loc(cell, var_i, var___ref_2);"),
        "c1: `b` takes the plain assignment"
    );
    assert!(
        !c1.contains("OpCopyRecord(cell,_src, var_b"),
        "c1: no deep copy into `b`"
    );
    assert!(
        c1.contains("if (var_b).store_nr != (var___ref_2).store_nr { OpFreeRef(cell,var_b"),
        "c1: `b`'s free is guarded by identity against the call's buffer"
    );
    assert!(
        !c1.contains("var___ref_2 = OpDatabase(cell,var___ref_2"),
        "c1: the paired buffer of a minting callee is not pooled at function entry"
    );
    assert!(
        c1.contains("var___ref_1 = OpDatabase(cell,var___ref_1"),
        "c1: the literal-returning callee's buffer keeps its pool"
    );
    // The destination that IS a return buffer keeps the in-place copy (plan 51 cluster 3).
    let lit = fn_body(&rust, "n_render_lit_then_call");
    assert!(
        lit.contains("OpCopyRecord(cell,_src, var_cv"),
        "render_lit_then_call: the promoted buffer local still copies its rebind in place"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_copy_on_native() {
    let out = std::env::temp_dir().join("loft_adopt_first_bind_off.rs");
    let rust = emit(&out, OFF);
    let c1 = fn_body(&rust, "n_c1");
    assert!(
        c1.contains("OpCopyRecord(cell,_src, var_b"),
        "c1 under LOFT_NO_ADOPT_FIRST_BIND: `b` is deep-copied"
    );
    assert!(
        !c1.contains("let mut var_b: DbRef = n_mk_loc("),
        "c1 under LOFT_NO_ADOPT_FIRST_BIND: no plain assignment"
    );
    let _ = std::fs::remove_file(&out);
}

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

#[test]
fn the_store_census_drops_by_one_per_adopting_bind() {
    // Hand-checked 2026-09-17 on the cells as written (the bind after an `if` pre-init adopts
    // too since then); a cell edit re-measures both pairs.  The native pair fell 101/137 →
    // 91/127 on 2026-09-18 with `@FR-R-LoopRecord`: a record literal bound inside a loop keeps
    // its store across the iterations, so ten per-pass mints are one — in BOTH arms, which is
    // why the adoption's own drop (36) is unchanged.  The interpreter's OFF count is 134 since the
    // arm became the scope (`@FR-B-Scope`, loft#1600, 2026-09-23; it read 139 before): c11's
    // `k = build(d - 1)` is its arm's local, a first bind, where the pre-init that used to stand in
    // front of the `if` made it a rebind that copies — one mint for each of the five activations
    // with `d > 0`.
    let (i_on, i_off) = (
        store_mints("--interpret", &[]),
        store_mints("--interpret", OFF),
    );
    assert!(
        i_on < i_off,
        "interpret: {i_on} mints with adoption, {i_off} without"
    );
    assert_eq!((i_on, i_off), (98, 134), "interpret mints (on, off)");
    let (n_on, n_off) = (store_mints("--native", &[]), store_mints("--native", OFF));
    assert!(
        n_on < n_off,
        "native: {n_on} mints with adoption, {n_off} without"
    );
    assert_eq!((n_on, n_off), (91, 127), "native mints (on, off)");
}
