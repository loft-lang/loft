// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 B2 (`@FR-R-Place`, `@FR-R-MoveLast`) — A RESULT BUILT WHERE IT WILL LIVE: a call
//! result whose one owning destination on every path that keeps it is a record-literal field
//! inside an element appended to a PARAMETER's collection gets its return buffer claimed in
//! that parameter's store, the field takes it by relocation at its last use, and the exit
//! releases the placed record alone.  The cell corpus
//! (`plans/164-activation-arena/bytecode-comparisons/B2-place-move-cells.loft`) says the
//! VALUES hold on both backends; this pins the IR SHAPE of the parse library's `add_poly`
//! (`OpPlaceRecord` at the buffer's init, `OpMoveRecord` at every field store, one
//! `OpFreeRecordIn` on the exit that still holds the record and none after a move, no copy
//! and no witness-guarded free left), that the switch
//! restores the copy shape, that the copy notice is silent on a relocated field and back
//! under the switch, and that the store census drops by exactly one per admitted call.
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
        .env_remove("LOFT_NO_PLACE_RESULT")
        .env_remove("LOFT_NO_LITERAL_EXIT_BUFFER")
        .env_remove("LOFT_NO_ADOPT_FIRST_BIND")
        .env_remove("LOFT_NO_VALUE_RETURN");
    cmd
}

/// The IR of `n_add_poly` as `loft introspect` prints it.
fn add_poly_ir(env: &[(&str, &str)]) -> String {
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
    let start = text.find("fn n_add_poly(").expect("add_poly's IR");
    let rest = &text[start..];
    let end = rest
        .find("}#block(1)")
        .expect("the function's closing block")
        + "}#block(1)".len();
    rest[..end].to_owned()
}

#[test]
fn add_poly_places_its_paint_in_the_scene_and_moves_it_at_every_store() {
    let ir = add_poly_ir(&[]);
    assert_eq!(
        ir.matches("= OpPlaceRecord(sc(0), ").count(),
        1,
        "the buffer's init must claim the record in the parameter's store:\n{ir}"
    );
    assert_eq!(
        ir.matches("OpMoveRecord(pp(").count(),
        2,
        "both stores of `pp` into the appended `Op` must relocate:\n{ir}"
    );
    assert_eq!(
        ir.matches("OpFreeRecordIn(pp(").count(),
        1,
        "only the exit that never stored the record releases it; the two exits after a move free nothing:\n{ir}"
    );
    assert!(
        !ir.contains("OpCopyRecord(pp(")
            && !ir.contains("OpFreeRefIfDistinct(")
            && !ir.contains("OpFreeRef(pp("),
        "no copy of `pp`, no store-level free and no witness-guarded free may remain:\n{ir}"
    );
}

#[test]
fn the_switch_restores_the_copy_shape() {
    let ir = add_poly_ir(&[("LOFT_NO_PLACE_RESULT", "1")]);
    assert!(
        ir.contains("__ref_1(1):ref(Paint) = null;"),
        "with the switch the buffer keeps its null init:\n{ir}"
    );
    assert_eq!(
        (
            ir.matches("OpCopyRecord(pp(").count(),
            ir.matches("OpFreeRefIfDistinct(__ref_1(1), pp(1))").count(),
            ir.matches("OpPlaceRecord").count()
                + ir.matches("OpMoveRecord").count()
                + ir.matches("OpFreeRecordIn").count(),
        ),
        (2, 3, 0),
        "with the switch off every field store copies and every exit takes the pair:\n{ir}"
    );
}

/// Run the cells in `mode` with `env`; (stdout, stderr) after asserting a clean exit.
fn run(mode: &str, env: &[(&str, &str)]) -> (String, String) {
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
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const STRICT: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
    ("LOFT_HOIST_VERIFY", "1"),
];
const STRICT_NATIVE: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
    ("LOFT_HOIST_VERIFY", "1"),
    ("LOFT_NATIVE_LEAK_CHECK", "1"),
];
const OFF: &[(&str, &str)] = &[("LOFT_NO_PLACE_RESULT", "1")];

#[test]
fn the_cells_hold_on_both_backends_and_under_the_switch() {
    let (on, _) = run("--interpret", STRICT);
    assert!(
        on.trim_end().ends_with("ok"),
        "cells did not reach `ok`:\n{on}"
    );
    let (native, _) = run("--native", STRICT_NATIVE);
    assert_eq!(on, native, "the two backends must print the same");
    let (off, _) = run("--interpret", OFF);
    assert_eq!(on, off, "the switch must not change a value");
    let (native_off, _) = run("--native", OFF);
    assert_eq!(
        on, native_off,
        "native under the switch must print the same"
    );
}

/// The copy notice (`advice[avoidable-copy]`) asks the pass for its verdict, so a field
/// the compiler relocates raises none — and under the switch the copies are real again and
/// the notice returns.
#[test]
fn the_copy_notice_is_silent_on_a_relocated_field_and_back_under_the_switch() {
    let (_, on) = run("--interpret", &[]);
    let (_, off) = run("--interpret", OFF);
    let count = |s: &str| s.matches("advice[avoidable-copy]").count();
    assert!(
        count(&on) < count(&off),
        "the notice must drop where a copy became a move: {} with placement, {} under the switch\n--- on ---\n{on}\n--- off ---\n{off}",
        count(&on),
        count(&off)
    );
}

/// `[db] OpDatabase` lines of `LOFT_TRACE_DB=1`: the store mints.
fn mints(mode: &str, env: &[(&str, &str)]) -> usize {
    let mut all: Vec<(&str, &str)> = vec![("LOFT_TRACE_DB", "1")];
    all.extend_from_slice(env);
    let (_, err) = run(mode, &all);
    err.lines()
        .filter(|l| l.starts_with("[db] ") && l.contains("OpDatabase"))
        .count()
}

/// The three admitted callees (`add_poly` ×64 through b1 and b4, `add_px` ×30, `add_card`
/// ×40) each mint a store per call under the switch and none with the placement, so the
/// interpreter's census drops by exactly their call count.  The interpreter is the oracle
/// here on purpose: native traces only its prefilling mint (`OpDatabase`), not the
/// complete-write `OpDatabaseNP` a no-heap literal such as `mk_rgb`'s takes, so its count
/// under the switch reads 30 low; native's parity is the value equality, the leak gate and
/// the IR shape pinned above.
#[test]
fn the_store_census_drops_by_one_per_admitted_call() {
    const ADMITTED_CALLS: usize = 4 + 60 + 30 + 40;
    let on = mints("--interpret", &[]);
    let off = mints("--interpret", OFF);
    assert_eq!(
        off - on,
        ADMITTED_CALLS,
        "interpreter: {off} mints under the switch, {on} with placement"
    );
}
