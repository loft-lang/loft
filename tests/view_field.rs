// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C5 (`@FR-O-ViewField`, `@FR-R-ValueRecord`) — a returned record's heap field as a
//! VIEW LEAF: the vector is delivered as a REFERENCE to the place it already lives in, so the
//! record's store and the deep copy into it are gone.
//!
//! `tests/scripts/164-view-field.loft` carries the cells and says the VALUES do not move; the
//! corpus runner runs it with the unit off, which is the record form the values are hand-
//! computed from.  This file carries the structural half, because the values cannot falsify an
//! admission: the copy the record form makes is correct, so every cell passes either way.  What
//! changes is the emitted signature and what the site reads, and that is what is pinned here —
//! together with every DECLINE, so one that quietly stops declining shows up as a failure here
//! rather than as a wrong answer in a consumer.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-view-field.loft";

fn guard() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_VIEW_FIELD");
    cmd
}

/// The generated Rust for the guard, with `env` applied.  `--native-emit` writes a file and
/// exits, which is the whole compilation this unit changes without running it.
fn emitted(env: &[(&str, &str)]) -> String {
    // One path per CALL, not per env: the tests run in parallel in one process, so two of
    // them sharing a name means one reads a file the other has already removed.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let out = std::env::temp_dir().join(format!(
        "loft_view_field_{}_{}.rs",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut cmd = loft();
    cmd.arg("--native-emit").arg(&out).arg(guard());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let run = cmd.output().expect("spawn loft");
    assert!(
        run.status.success(),
        "--native-emit failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let text = std::fs::read_to_string(&out).expect("the emitted source");
    let _ = std::fs::remove_file(&out);
    text
}

/// One function's emitted SIGNATURE line.
fn signature(src: &str, func: &str) -> String {
    let head = format!("fn n_{func}(");
    src.lines()
        .find(|l| l.starts_with(&head))
        .unwrap_or_else(|| panic!("{func} is not in the emission"))
        .to_owned()
}

const ON: &[(&str, &str)] = &[("LOFT_VIEW_FIELD", "1")];

/// The admitted shape: two scalars and a REFERENCE, with the caller's buffer argument gone.
#[test]
fn an_admitted_function_returns_its_heap_field_as_a_reference() {
    let src = emitted(ON);
    for func in ["a1", "a2", "a3", "a4", "a5", "c1f", "c2f"] {
        let sig = signature(&src, func);
        assert!(
            sig.contains("-> (bool, bool, DbRef)"),
            "{func} must answer the value tuple with a view leaf:\n{sig}"
        );
        assert!(
            !sig.contains("__retbuf"),
            "{func} must drop the caller's return buffer:\n{sig}"
        );
    }
}

/// And the SITE reads the tuple's own reference where it read a field of a record.
#[test]
fn a_site_reads_the_leaf_off_the_tuple() {
    let src = emitted(ON);
    assert!(
        src.contains("let mut var_m1: (bool, bool, DbRef) = n_a1(cell, var_s1, 3_i64);"),
        "the site binds the tuple and passes no buffer"
    );
    assert!(
        src.contains("t_6vector_len(cell, var_m1.2)"),
        "a length reads the leaf's reference straight off the tuple"
    );
}

/// The switch restores the record form whole — the unit's own control.
#[test]
fn the_unit_is_off_by_default_and_the_switch_restores_the_record() {
    for env in [
        &[][..],
        &[("LOFT_VIEW_FIELD", "1"), ("LOFT_NO_VIEW_FIELD", "1")][..],
    ] {
        let src = emitted(env);
        let sig = signature(&src, "a1");
        assert!(
            sig.contains("var___retbuf: DbRef) -> DbRef"),
            "without the unit a1 keeps its return buffer:\n{sig}"
        );
    }
}

/// Every decline, by the condition it stands for.  A decline that quietly stops declining is
/// this unit's whole risk: three of these would be SILENT in a consumer.
#[test]
fn every_decline_keeps_the_record_form() {
    let src = emitted(ON);
    for (func, why) in [
        ("d1", "the source is owned by the frame"),
        (
            "d2",
            "the source is a `?`-discharge, whose ownership is a join",
        ),
        ("d3", "the source has two owning destinations"),
        ("d4", "the callee grows the viewed container again"),
        ("d5", "a text field is not a leaf type"),
        (
            "d6",
            "the field is filled by a literal, which pushes element by element",
        ),
        ("b1f", "a site appends to the field"),
        (
            "b2f",
            "a site grows the container between the bind and the read",
        ),
        ("b3f", "a site grows it through a call"),
        ("b4f", "a site keeps the record"),
        ("b5f", "a site reads before the bind in a loop"),
        ("b6f", "a site iterates the field"),
        (
            "b7f",
            "a site reads an element of the field and writes through it",
        ),
        (
            "b8f",
            "a site removes from the container between the bind and the read",
        ),
    ] {
        let sig = signature(&src, func);
        assert!(
            sig.contains("var___retbuf: DbRef) -> DbRef"),
            "{func} must keep the record form — {why}:\n{sig}"
        );
    }
}

/// The cells' values, on both backends and under the store falsifiers: a view leaf is never
/// freed and never read after its place moved.
#[test]
fn the_cells_hold_on_both_backends_under_every_falsifier() {
    let strict: &[(&str, &str)] = &[
        ("LOFT_POISON", "1"),
        ("LOFT_POISON_CLAIM", "1"),
        ("LOFT_STRICT_STORES", "1"),
        ("LOFT_NATIVE_LEAK_CHECK", "1"),
    ];
    for backend in ["--interpret", "--native"] {
        let mut cmd = loft();
        cmd.arg(backend).arg(guard()).env("LOFT_VIEW_FIELD", "1");
        for (k, v) in strict {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("spawn loft");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && text.contains("164-view-field: every cell holds"),
            "{backend} did not hold under the falsifiers:\n{text}"
        );
        assert!(
            !text.contains("stores not freed"),
            "{backend} leaked:\n{text}"
        );
    }
}
