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
