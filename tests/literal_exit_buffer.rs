// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 B2, the callee clause of `@FR-R-Place` — EVERY LITERAL EXIT WRITES THE HANDED
//! BUFFER: a mid-body `return S { … }` builds its record into the `__retbuf` the caller
//! handed, exactly as the tail literal does, instead of minting a per-exit store of its own.
//! A callee that answered a different store per exit could never be handed a record that
//! lives where its result is going, which is what (R-Place) needs.  The cell corpus
//! (`plans/164-activation-arena/bytecode-comparisons/B2-place-move-cells.loft`) says the
//! VALUES hold on both backends; this pins the IR SHAPE — `read_paint`'s four exits all
//! target `__retbuf` and none its `__ref_p2_N` work-ref — and that the switch restores the
//! per-exit stores with the same values.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/B2-place-move-cells.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "200")
        .env_remove("LOFT_NO_LITERAL_EXIT_BUFFER")
        .env_remove("LOFT_NO_VALUE_RETURN");
    cmd
}

/// The IR of `n_read_paint` as `loft introspect` prints it.
fn read_paint_ir(env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect").arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find("fn n_read_paint(").expect("read_paint's IR");
    let rest = &text[start..];
    let end = rest
        .find("}#block(1)")
        .expect("the function's closing block")
        + "}#block(1)".len();
    rest[..end].to_owned()
}

#[test]
fn every_literal_exit_of_read_paint_writes_the_handed_buffer() {
    let ir = read_paint_ir(&[]);
    let into_buffer = ir.matches("#Object(").count() / 2;
    let targeted = ir.matches(":ref(Paint)[\"__retbuf\"]").count() / 2;
    assert_eq!(
        (into_buffer, targeted),
        (4, 4),
        "read_paint has four literal exits and every one must build into `__retbuf`:\n{ir}"
    );
    assert!(
        !ir.contains("[\"__ref_p2_"),
        "no exit may keep a per-exit work-ref as its destination:\n{ir}"
    );
}

#[test]
fn the_switch_restores_the_per_exit_stores() {
    let ir = read_paint_ir(&[("LOFT_NO_LITERAL_EXIT_BUFFER", "1")]);
    let tail_only = ir.matches(":ref(Paint)[\"__retbuf\"]").count() / 2;
    let per_exit = ir.matches("[\"__ref_p2_").count() / 2;
    assert_eq!(
        (tail_only, per_exit),
        (1, 3),
        "with the switch off only the tail targets `__retbuf` and the three mid-body \
         exits keep their own work-ref:\n{ir}"
    );
}

const B1_GUARD: &str = "tests/scripts/164-adopt-first-bind.loft";

#[test]
fn a_promoted_local_beside_a_literal_exit_keeps_its_per_exit_store() {
    // `mk_mix`: `o = P { … }; if i % 2 == 0 { return P { … } }; …; o` — the promoted local
    // IS the buffer, so the early literal must NOT be built into it (the two would share
    // one record and the caller freed a stale ref: the corpus's `BUG (#306)` refusal).
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(B1_GUARD));
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find("fn n_mk_mix(").expect("mk_mix's IR");
    let rest = &text[start..];
    let ir = &rest[..rest.find("}#block(1)").expect("closing block")];
    assert!(
        ir.contains("[\"__ref_p2_"),
        "mk_mix's early literal exit must keep its own work-ref:\n{ir}"
    );
}

/// Run the cells in `mode` with `env`; the stdout, after asserting a clean exit.
fn run(mode: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg(mode).arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{mode} {env:?} failed (exit {:?}):\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const STRICT: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
];
const STRICT_NATIVE: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
    ("LOFT_NATIVE_LEAK_CHECK", "1"),
];
const OFF: &[(&str, &str)] = &[("LOFT_NO_LITERAL_EXIT_BUFFER", "1")];

#[test]
fn the_cells_hold_on_both_backends_and_under_the_switch() {
    let on = run("--interpret", STRICT);
    assert!(
        on.trim_end().ends_with("ok"),
        "cells did not reach `ok`:\n{on}"
    );
    let native = run("--native", STRICT_NATIVE);
    assert_eq!(on, native, "the two backends must print the same");
    let off = run("--interpret", OFF);
    assert_eq!(on, off, "the switch must not change a value");
    let native_off = run("--native", OFF);
    assert_eq!(
        on, native_off,
        "native under the switch must print the same"
    );
}

const CHAIN_GUARD: &str = "tests/scripts/164-a-literal-exit-writes-its-chains-buffer.loft";

/// The listing of `fn <name>(` in `text` up to the function's closing block.
fn function_ir<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("{name}'s IR"));
    let rest = &text[start..];
    &rest[..rest.find("}#block(1)").expect("closing block")]
}

fn chain_guard_ir(env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CHAIN_GUARD));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_literal_exit_beside_a_chain_writes_the_chains_buffer() {
    // A chain renames the buffer after its work ref; a literal exit beside it builds into
    // that buffer wherever the chain stands — mid-body and tail (`scan`), tail only
    // (`outer2`), mid-body only with a literal TAIL (`tail_lit`).
    let text = chain_guard_ir(&[]);
    for (name, literal) in [
        ("n_scan", "return {#Object(8):ref(Mk)[\"__ref_1\"]"),
        ("n_outer2", "return {#Object(3):ref(Mk)[\"__ref_1\"]"),
        ("n_tail_lit", "return {#Object(6):ref(Mk)[\"__ref_1\"]"),
    ] {
        let ir = function_ir(&text, name);
        assert!(
            ir.contains(literal),
            "{name}: the literal builds into the chain's buffer:\n{ir}"
        );
        assert!(
            !ir.contains("OpDatabase(__ref_p2_"),
            "{name}: no literal mints a store of its own:\n{ir}"
        );
    }
    // A promoted NAMED local is not a chain's buffer: `pick`'s literal keeps its own store.
    assert!(
        function_ir(&text, "n_pick").contains("OpDatabase(__ref_p2_"),
        "pick: the literal beside a promoted local keeps its own store"
    );
    // The switch restores the per-exit store.
    let off = chain_guard_ir(OFF);
    assert!(
        function_ir(&off, "n_scan").contains("OpDatabase(__ref_p2_"),
        "LOFT_NO_LITERAL_EXIT_BUFFER: scan's literal mints its own store again"
    );
}

#[test]
fn a_scanner_beside_a_chain_keeps_its_registers() {
    // The chain's buffer is a PHANTOM value local on native; the literal's writes into it
    // vanish with the converted `Object`, so both shapes stay value records.
    let out = std::env::temp_dir().join("loft_literal_exit_chain.rs");
    let status = loft()
        .arg("--native-emit")
        .arg(&out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CHAIN_GUARD))
        .output()
        .expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    for sig in [
        "fn n_find_w(cell: &std::cell::UnsafeCell<Stores>, mut var_s: &str, mut var_c: i64, mut var_start: i64) -> (bool, i64, f64)",
        "fn n_digits(cell: &std::cell::UnsafeCell<Stores>, mut var_s: &str, mut var_i: i64) -> (bool, i64, f64)",
        "fn n_either(cell: &std::cell::UnsafeCell<Stores>, mut var_s: &str, mut var_i: i64) -> (bool, i64, f64)",
    ] {
        assert!(rust.contains(sig), "still a value record: {sig}");
    }
    let _ = std::fs::remove_file(&out);
}
